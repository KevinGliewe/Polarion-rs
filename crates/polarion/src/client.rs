use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use polarion_soap::Auth;

use crate::activities::Activities;
use crate::baselines::Baselines;
use crate::builds::Builds;
use crate::documents::Documents;
use crate::error::{Error, Result};
use crate::folders::Folders;
use crate::items::WorkItems;
use crate::plans::Plans;
use crate::projects::{Projects, Users};
use crate::restricted;
use crate::revisions::Revisions;
use crate::schema::Schema;
use crate::security::Security;
use crate::tests_mgmt::TestRuns;
use crate::uri;
use crate::wiki::Wiki;

/// Why a write-restricted client refuses a write that may change objects it cannot see.
const UNBOUNDED: &str = "the client writes only to its write targets, and this write may change other objects or cannot be undone";
/// Why a write-restricted client refuses a write that is only checked against projects.
const UNCHECKED: &str =
    "the client writes only to its write targets, and this write is not checked against them";
/// Why a write-restricted client refuses a status or resolution change.
const STATUS: &str =
    "the client writes only to its write targets and never changes `status` or `resolution`";
/// Why a write-restricted client refuses a create.
const NO_CREATE: &str = "the client's create budget has no create of this kind left";
/// Why a write-restricted client without targets or creates refuses a transaction.
const NOTHING_WRITABLE: &str = "the client writes only to its write targets and has none";
/// Why a write-restricted client refuses a document's structure link role.
pub(crate) const DOCUMENT_STRUCTURE: &str = "the client writes only to its write targets and never changes a document's structure link role, which changes the links of its work items";
/// Why a write-restricted client refuses a document body with work item macros.
pub(crate) const DOCUMENT_MACROS: &str = "the client writes only to its write targets and never writes a document body with work item macros (`module-workitem`), which change the work items of the document";
/// Why a write-restricted client refuses the body or outline numbering of a document with work
/// items.
pub(crate) const DOCUMENT_ITEMS: &str = "the client writes only to its write targets and writes the body and outline numbering only of documents without work items";

/// Field keys a write-restricted client never writes: they change with workflow actions.
const STATUS_KEYS: [&str; 2] = ["status", "resolution"];

/// Convenient access to a Polarion server, built on the SOAP web services.
///
/// Operations are grouped by area: [`Polarion::items`], [`Polarion::documents`],
/// [`Polarion::schema`], [`Polarion::projects`] and [`Polarion::users`].
///
/// **Read-only by default.** Writes must be enabled per project with
/// [`PolarionBuilder::allow_writes`]; every write method checks the target's
/// project against that list before anything is sent.
///
/// **Restricted to write targets** with [`PolarionBuilder::restrict_writes`] (and
/// [`PolarionBuilder::allow_creates`]): the client then only sends writes to the listed objects.
/// Writes that may change other objects or cannot be undone are refused, and so is every request
/// the SOAP write filter does not recognize as such a write. What the server itself does in
/// response (suspect links on work items that link to a target, notifications) is not prevented
/// (see [`PolarionBuilder::restrict_writes`]).
///
/// Cheap to clone; clones share the session, which is opened on the first call.
#[derive(Debug, Clone)]
pub struct Polarion {
    soap: polarion_soap::Client,
    writes: Arc<WritePolicy>,
}

/// How many objects of each kind a write-restricted client may create
/// ([`PolarionBuilder::allow_creates`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreateBudget {
    pub work_items: u32,
    pub test_runs: u32,
    pub documents: u32,
}

/// The kinds of objects a [`CreateBudget`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateKind {
    WorkItem,
    TestRun,
    Document,
}

/// What is left of a [`CreateBudget`]: one counter per kind, for one project.
#[derive(Debug)]
pub(crate) struct Creates {
    pub(crate) project: String,
    pub(crate) budget: CreateBudget,
    work_items: AtomicU32,
    test_runs: AtomicU32,
    documents: AtomicU32,
}

impl Creates {
    pub(crate) fn new(project: &str, budget: CreateBudget) -> Self {
        Self {
            project: project.to_owned(),
            budget,
            work_items: AtomicU32::new(budget.work_items),
            test_runs: AtomicU32::new(budget.test_runs),
            documents: AtomicU32::new(budget.documents),
        }
    }

    /// Take one create of `kind`; `false` when none is left.
    pub(crate) fn take(&self, kind: CreateKind) -> bool {
        let counter = match kind {
            CreateKind::WorkItem => &self.work_items,
            CreateKind::TestRun => &self.test_runs,
            CreateKind::Document => &self.documents,
        };
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }

    /// Whether the budget allowed any create at all.
    fn any(&self) -> bool {
        self.budget != CreateBudget::default()
    }
}

#[derive(Debug, Default)]
struct WritePolicy {
    projects: BTreeSet<String>,
    global: bool,
    /// Restricted mode: the objects writes may change (URIs without `%revision`).
    targets: Option<BTreeSet<String>>,
    /// Restricted mode: what may be created.
    creates: Option<Creates>,
    /// Restricted mode: the document writes the guard checked, for the SOAP write filter.
    approvals: Arc<restricted::Approvals>,
}

impl WritePolicy {
    /// Restricted mode: on when targets or a create budget were given, even empty ones.
    fn restricted(&self) -> bool {
        self.targets.is_some() || self.creates.is_some()
    }

    fn is_target(&self, uri: &str) -> bool {
        self.targets
            .as_ref()
            .is_some_and(|t| t.contains(uri::strip_revision(uri)))
    }

    /// Whether a restricted client can write anything at all.
    fn restricted_can_write(&self) -> bool {
        self.targets.as_ref().is_some_and(|t| !t.is_empty())
            || self.creates.as_ref().is_some_and(Creates::any)
    }
}

impl Polarion {
    /// A read-only client for `base_url` (e.g. `https://polarion.example.com/polarion`).
    pub fn new(base_url: &str, auth: Auth) -> Result<Self> {
        Self::builder(base_url, auth).build()
    }

    /// Configure TLS, timeouts and writes.
    pub fn builder(base_url: &str, auth: Auth) -> PolarionBuilder {
        PolarionBuilder {
            soap: polarion_soap::Client::builder(base_url, auth),
            writes: WritePolicy::default(),
        }
    }

    /// Wrap an existing SOAP client. Writes through this library are refused
    /// (the client's own read-only setting still applies to [`Polarion::soap`]).
    pub fn from_client(soap: polarion_soap::Client) -> Self {
        Self {
            soap,
            writes: Arc::default(),
        }
    }

    /// The underlying SOAP client, for operations this library does not cover.
    ///
    /// Calls made through it are **not** checked against the write allowlist. A client
    /// restricted to write targets ([`PolarionBuilder::restrict_writes`]) still checks them with
    /// its SOAP write filter.
    pub fn soap(&self) -> &polarion_soap::Client {
        &self.soap
    }

    /// Projects this client may write to (none when it is restricted to write targets).
    pub fn writable_projects(&self) -> impl Iterator<Item = &str> {
        let restricted = self.writes.restricted();
        self.writes
            .projects
            .iter()
            .filter(move |_| !restricted)
            .map(String::as_str)
    }

    /// Whether writes not tied to a project (users, global folders, ...) are allowed (never when
    /// the client is restricted to write targets).
    pub fn global_writes_allowed(&self) -> bool {
        self.writes.global && !self.writes.restricted()
    }

    /// Whether the client is restricted to write targets ([`PolarionBuilder::restrict_writes`],
    /// [`PolarionBuilder::allow_creates`]).
    pub fn writes_restricted(&self) -> bool {
        self.writes.restricted()
    }

    /// The write targets (object URIs without `%revision`), in restricted mode.
    pub fn write_targets(&self) -> impl Iterator<Item = &str> {
        self.writes.targets.iter().flatten().map(String::as_str)
    }

    /// Whether a write to the object `uri` would pass the guard (sends nothing): in restricted
    /// mode, whether `uri` (without `%revision`) is a write target; otherwise whether its
    /// project may be written to. A write method may still refuse it for its other arguments or
    /// because of what it does.
    pub fn write_allowed(&self, uri: &str) -> bool {
        if self.writes.restricted() {
            self.writes.is_target(uri)
        } else {
            uri::project_of(uri).is_some_and(|p| self.writes.projects.contains(p))
        }
    }

    /// End the session on the server.
    pub async fn logout(&self) -> Result<()> {
        Ok(self.soap.logout().await?)
    }

    pub fn items(&self) -> WorkItems<'_> {
        WorkItems::new(self)
    }

    pub fn documents(&self) -> Documents<'_> {
        Documents::new(self)
    }

    pub fn schema(&self) -> Schema<'_> {
        Schema::new(self)
    }

    pub fn folders(&self) -> Folders<'_> {
        Folders::new(self)
    }

    pub fn wiki(&self) -> Wiki<'_> {
        Wiki::new(self)
    }

    pub fn baselines(&self) -> Baselines<'_> {
        Baselines::new(self)
    }

    pub fn revisions(&self) -> Revisions<'_> {
        Revisions::new(self)
    }

    pub fn activities(&self) -> Activities<'_> {
        Activities::new(self)
    }

    /// Test management: test runs, records, steps.
    pub fn tests(&self) -> TestRuns<'_> {
        TestRuns::new(self)
    }

    pub fn plans(&self) -> Plans<'_> {
        Plans::new(self)
    }

    pub fn builds(&self) -> Builds<'_> {
        Builds::new(self)
    }

    /// Permissions, roles and licensing.
    pub fn security(&self) -> Security<'_> {
        Security::new(self)
    }

    pub fn projects(&self) -> Projects<'_> {
        Projects::new(self)
    }

    pub fn users(&self) -> Users<'_> {
        Users::new(self)
    }

    // ---- write guards: every write method calls one before it sends anything

    pub(crate) fn refused(
        operation: &'static str,
        project: Option<&str>,
        target: Option<&str>,
        reason: Option<&'static str>,
    ) -> Error {
        Error::WriteNotAllowed {
            operation,
            project: project.map(str::to_owned),
            target: target.map(str::to_owned),
            reason,
        }
    }

    /// Refuse a write unless `project` is in the allowlist. Refuses everything in restricted
    /// mode, where only the target guards ([`Polarion::check_write_target`] and friends) allow
    /// writes.
    pub(crate) fn check_write(&self, operation: &'static str, project: Option<&str>) -> Result<()> {
        if self.writes.restricted() {
            return Err(Self::refused(operation, None, None, Some(UNCHECKED)));
        }
        match project {
            Some(p) if self.writes.projects.contains(p) => Ok(()),
            _ => Err(Self::refused(operation, project, None, None)),
        }
    }

    /// [`Polarion::check_write`] for the project of an object URI.
    pub(crate) fn check_write_uri(&self, operation: &'static str, uri: &str) -> Result<()> {
        self.check_write(operation, uri::project_of(uri))
    }

    /// Refuse a write that is not tied to a project unless global writes are allowed (never in
    /// restricted mode).
    pub(crate) fn check_global_write(&self, operation: &'static str) -> Result<()> {
        if self.writes.restricted() {
            Err(Self::refused(operation, None, None, Some(UNCHECKED)))
        } else if self.writes.global {
            Ok(())
        } else {
            Err(Self::refused(operation, None, None, None))
        }
    }

    /// A project write when `project` is given, a global write otherwise.
    pub(crate) fn check_write_scope(
        &self,
        operation: &'static str,
        project: Option<&str>,
    ) -> Result<()> {
        match project {
            Some(p) => self.check_write(operation, Some(p)),
            None => self.check_global_write(operation),
        }
    }

    /// Refuse unless some writes are allowed (never in restricted mode).
    pub(crate) fn check_any_write(&self, operation: &'static str) -> Result<()> {
        if self.writes.restricted() {
            Err(Self::refused(operation, None, None, Some(UNCHECKED)))
        } else if self.writes.global || !self.writes.projects.is_empty() {
            Ok(())
        } else {
            Err(Self::refused(operation, None, None, None))
        }
    }

    fn require_target(&self, operation: &'static str, uri: &str) -> Result<()> {
        if self.writes.is_target(uri) {
            Ok(())
        } else {
            Err(Self::refused(operation, None, Some(uri), None))
        }
    }

    /// A write to the existing object `uri`, the URI that is sent. Restricted mode: `uri` must
    /// be a write target. Otherwise: its project must be writable.
    pub(crate) fn check_write_target(&self, operation: &'static str, uri: &str) -> Result<()> {
        if self.writes.restricted() {
            self.require_target(operation, uri)
        } else {
            self.check_write_uri(operation, uri)
        }
    }

    /// A write to `uri` that names further objects (`named`: link targets, test cases, defects,
    /// plan items). Restricted mode: all of them must be write targets. Otherwise only the
    /// project of `uri` is checked (links, for example, may point to other projects).
    pub(crate) fn check_write_targets(
        &self,
        operation: &'static str,
        uri: &str,
        named: &[&str],
    ) -> Result<()> {
        if self.writes.restricted() {
            self.require_target(operation, uri)?;
            named
                .iter()
                .try_for_each(|n| self.require_target(operation, n))
        } else {
            self.check_write_uri(operation, uri)
        }
    }

    /// A write to an object of `owner` (a comment, a work record) addressed by its own URI
    /// `owned`. Restricted mode: `owner` must be a write target, and [`uri::owner_of`] must read
    /// that same owner from `owned` (an unknown format is refused). Otherwise: the projects of
    /// both URIs must be writable.
    pub(crate) fn check_write_owned(
        &self,
        operation: &'static str,
        owner: &str,
        owned: &str,
    ) -> Result<()> {
        if self.writes.restricted() {
            self.require_target(operation, owner)?;
            match uri::owner_of(owned) {
                Some(o) if o == uri::strip_revision(owner) => Ok(()),
                _ => Err(Self::refused(operation, None, Some(owned), None)),
            }
        } else {
            self.check_write_uri(operation, owner)?;
            self.check_write_uri(operation, owned)
        }
    }

    /// A create in `project`. Restricted mode: only in the create budget's project, while the
    /// budget has a create of this kind left; every attempt allowed here takes one, even if it
    /// fails later. Otherwise: the project must be writable.
    pub(crate) fn check_create(
        &self,
        operation: &'static str,
        project: &str,
        kind: CreateKind,
    ) -> Result<()> {
        if !self.writes.restricted() {
            return self.check_write(operation, Some(project));
        }
        match &self.writes.creates {
            Some(c) if c.project == project && c.take(kind) => Ok(()),
            _ => Err(Self::refused(
                operation,
                Some(project),
                None,
                Some(NO_CREATE),
            )),
        }
    }

    /// A write that may change objects the guard cannot see, or cannot be undone (workflow
    /// actions, deletes, moves, reuse, creates outside the budget, users, ...): always refused
    /// in restricted mode. Otherwise the method's project guard follows.
    pub(crate) fn check_unbounded_write(&self, operation: &'static str) -> Result<()> {
        if self.writes.restricted() {
            Err(Self::refused(operation, None, None, Some(UNBOUNDED)))
        } else {
            Ok(())
        }
    }

    /// Restricted mode: refuse writing the fields `status` and `resolution` (any case), which
    /// change with workflow actions. Nothing to check otherwise.
    pub(crate) fn check_field_keys<'k>(
        &self,
        operation: &'static str,
        keys: impl IntoIterator<Item = &'k str>,
    ) -> Result<()> {
        if self.writes.restricted() && keys.into_iter().any(is_status_key) {
            Err(Self::refused(operation, None, None, Some(STATUS)))
        } else {
            Ok(())
        }
    }

    /// Restricted mode: the guard checked a write of the body or outline numbering of the
    /// document `uri` (it holds no work items); the SOAP write filter lets one such request
    /// through.
    pub(crate) fn approve_document_write(&self, uri: &str) {
        self.writes.approvals.approve(uri);
    }

    /// Transactions only wrap writes that are checked one by one: allowed in restricted mode
    /// when the client can write anything at all; otherwise when some writes are allowed.
    fn check_transaction(&self, operation: &'static str) -> Result<()> {
        if !self.writes.restricted() {
            self.check_any_write(operation)
        } else if self.writes.restricted_can_write() {
            Ok(())
        } else {
            Err(Self::refused(operation, None, None, Some(NOTHING_WRITABLE)))
        }
    }

    // ---- session

    /// Log in now instead of on the first call (e.g. to fail fast on bad credentials).
    pub async fn login(&self) -> Result<()> {
        Ok(self.soap.login().await?)
    }

    /// Whether the session has a logged-in user (`hasSubject`).
    pub async fn has_session(&self) -> Result<bool> {
        Ok(self.soap.session().has_subject().await?)
    }

    /// Start an explicit transaction: the following changes are committed together by
    /// [`Polarion::commit`] or discarded by [`Polarion::rollback`]. Needs writes to be enabled.
    pub async fn begin_transaction(&self) -> Result<()> {
        self.check_transaction("beginTransaction")?;
        Ok(self.soap.session().begin_transaction().await?)
    }

    pub async fn commit(&self) -> Result<()> {
        self.check_transaction("endTransaction")?;
        Ok(self.soap.session().end_transaction(false).await?)
    }

    pub async fn rollback(&self) -> Result<()> {
        self.check_transaction("endTransaction")?;
        Ok(self.soap.session().end_transaction(true).await?)
    }

    /// Whether an explicit transaction is open.
    pub async fn in_transaction(&self) -> Result<bool> {
        Ok(self.soap.session().transaction_exists().await?)
    }
}

/// Whether a field key is `status` or `resolution` (any case, surrounding white space ignored).
pub(crate) fn is_status_key(key: &str) -> bool {
    STATUS_KEYS
        .iter()
        .any(|k| key.trim().eq_ignore_ascii_case(k))
}

/// Builder for [`Polarion`].
#[derive(Debug)]
pub struct PolarionBuilder {
    soap: polarion_soap::ClientBuilder,
    writes: WritePolicy,
}

impl PolarionBuilder {
    /// Per-request timeout (default: 120 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.soap = self.soap.timeout(timeout);
        self
    }

    /// Trust an additional CA certificate (PEM). The OS trust store is used by default.
    pub fn add_root_certificate_pem(mut self, pem: &[u8]) -> Result<Self> {
        self.soap = self.soap.add_root_certificate_pem(pem)?;
        Ok(self)
    }

    /// Disable TLS certificate validation. Only for test instances with self-signed certificates.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.soap = self.soap.danger_accept_invalid_certs(accept);
        self
    }

    /// Allow writes to these projects (by id). Writes to any other project are
    /// refused before they are sent. Without this the client is read-only.
    /// Has no effect on a client restricted to write targets ([`PolarionBuilder::restrict_writes`]).
    pub fn allow_writes<I, S>(mut self, projects: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.writes.projects.extend(
            projects
                .into_iter()
                .map(Into::into)
                .filter(|p| !p.is_empty()),
        );
        self
    }

    /// Allow writes that are not tied to a project: creating and updating users,
    /// global folders, comments on activities, the user account vault.
    /// Has no effect on a client restricted to write targets ([`PolarionBuilder::restrict_writes`]).
    pub fn allow_global_writes(mut self, allow: bool) -> Self {
        self.writes.global = allow;
        self
    }

    /// Restrict writes to these objects, by URI (a `%revision` suffix is ignored), e.g. the
    /// `uri` of fetched work items, documents and test runs. Calling it again adds targets.
    ///
    /// This turns on **restricted mode** (so does [`PolarionBuilder::allow_creates`]), even with
    /// no targets at all; [`PolarionBuilder::allow_writes`] and
    /// [`PolarionBuilder::allow_global_writes`] then have no effect. Before anything is sent:
    /// - a write to an existing object is allowed only when every object it changes or names is
    ///   a target (link targets, test cases, defects, plan items, the user a test record names
    ///   included); comments and work records count as their owner's
    ///   ([`crate::uri::owner_of`]), and must belong to the object the method is given;
    /// - creates are allowed only within the budget of [`PolarionBuilder::allow_creates`];
    /// - `status` and `resolution` are never written (work items, documents, test runs, plans);
    /// - documents: the structure link role is never written, a body never with work item
    ///   macros, and the body and outline numbering only of documents that hold no work items
    ///   (the guard reads that before the write);
    /// - writes that may change other objects or cannot be undone are always refused: workflow
    ///   actions, deletes of whole objects, moves, reuse and derived documents, test executions
    ///   and summary defects, project spans, external, OSLC and revision links, auto-suspect and
    ///   auto-assign, other creates, users, folders, the account vault, activity comments;
    /// - deletes of what belongs to a target are allowed: its attachments, work records, test
    ///   records (`TestRuns::remove_record`) and their attachments;
    /// - transactions are allowed (they wrap writes that are checked one by one).
    ///
    /// Refusals are [`Error::WriteNotAllowed`]. In addition, the SOAP client gets a write
    /// filter, which also covers [`Polarion::soap`]: it parses each write request and sends it
    /// only for the operations this library allows in restricted mode, only when the parameter
    /// that names the object the operation changes is a target (or, for comments and work
    /// records, owned by one), and only when every object URI in the request is a target, owned
    /// by that target, or (for a budgeted create) the budget's project; markup the request
    /// writer never emits, unknown or repeated parameters and identity fields in updated objects
    /// are refused. It allows at most the budget's number of create requests of each kind, a
    /// create repeated after a session renewal included. Other writes fail with
    /// [`polarion_soap::Error::WriteFiltered`]. The SOAP client is read-only when there are
    /// neither targets nor creates.
    ///
    /// **What it does not prevent:** what the server does in response to a write to a target.
    /// Changing a work item can mark links from other work items to it as suspect (their
    /// revisions change), and notification rules may email people about any change. Check the
    /// back links of a work item target ([`crate::WorkItems::back_links`]) and the project's
    /// notification settings before relying on "only the targets change".
    pub fn restrict_writes<I, S>(mut self, uris: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let targets = self.writes.targets.get_or_insert_with(BTreeSet::new);
        for uri in uris {
            let uri = uri.into();
            let uri = uri::strip_revision(&uri);
            if !uri.is_empty() {
                targets.insert(uri.to_owned());
            }
        }
        self
    }

    /// Allow creating up to `budget` work items, test runs and documents in `project`, and
    /// nothing else (`createWorkItem`, `createDocument`, `createTestRun`,
    /// `createTestRunWithTitle`). Turns on restricted mode (see
    /// [`PolarionBuilder::restrict_writes`]). Every create the guard lets through takes one from
    /// its kind's budget, even if it fails later. Calling it again replaces the budget.
    pub fn allow_creates(mut self, project: &str, budget: CreateBudget) -> Self {
        self.writes.creates = Some(Creates::new(project, budget));
        self
    }

    pub fn build(self) -> Result<Polarion> {
        let writes = self.writes;
        let soap = if writes.restricted() {
            let filter = restricted::Filter::new(
                writes.targets.as_ref(),
                writes.creates.as_ref(),
                writes.approvals.clone(),
            );
            self.soap
                .read_only(!writes.restricted_can_write())
                .write_filter(Arc::new(move |operation, body| {
                    filter.allows(operation, body)
                }))
        } else {
            self.soap
                .read_only(writes.projects.is_empty() && !writes.global)
        };
        Ok(Polarion {
            soap: soap.build()?,
            writes: Arc::new(writes),
        })
    }
}

pub(crate) fn not_found(kind: &'static str, id: impl std::fmt::Display) -> Error {
    Error::NotFound {
        kind,
        id: id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: &str = "subterra:data-service:objects:/default/Sandbox${WorkItem}SB-1";

    fn client(builder: impl FnOnce(PolarionBuilder) -> PolarionBuilder) -> Polarion {
        builder(Polarion::builder(
            "https://polarion.invalid/polarion",
            Auth::Token("t".into()),
        ))
        .build()
        .unwrap()
    }

    fn refused(result: Result<()>) -> (Option<String>, Option<String>, Option<&'static str>) {
        match result {
            Err(Error::WriteNotAllowed {
                project,
                target,
                reason,
                ..
            }) => (project, target, reason),
            other => panic!("expected WriteNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn project_guards_refuse_everything_in_restricted_mode() {
        let polarion = client(|b| {
            b.allow_writes(["Sandbox"])
                .allow_global_writes(true)
                .restrict_writes([ITEM])
        });
        for result in [
            polarion.check_write("op", Some("Sandbox")),
            polarion.check_write_uri("op", ITEM),
            polarion.check_write_scope("op", Some("Sandbox")),
            polarion.check_write_scope("op", None),
            polarion.check_global_write("op"),
            polarion.check_any_write("op"),
            polarion.check_unbounded_write("op"),
        ] {
            let (project, target, reason) = refused(result);
            assert_eq!((project, target), (None, None));
            assert!(reason.is_some());
        }
        polarion.check_write_target("op", ITEM).unwrap();
        polarion.check_transaction("op").unwrap();
        assert_eq!(
            refused(polarion.check_create("op", "Sandbox", CreateKind::WorkItem)).0,
            Some("Sandbox".to_owned())
        );
    }

    #[test]
    fn owned_objects_belong_to_the_given_target() {
        let other = "subterra:data-service:objects:/default/Sandbox${WorkItem}SB-2";
        let polarion = client(|b| b.restrict_writes([ITEM, other]));
        // The formats Polarion uses: `…${Comment}{WorkItem}ID#KEY`, `…${WorkRecord}{WorkItem}ID#KEY`.
        let comment = |item: &str, key: &str| {
            format!(
                "{}#{key}",
                item.replacen("${WorkItem}", "${Comment}{WorkItem}", 1)
            )
        };
        let record = |item: &str, key: &str| {
            format!(
                "{}#{key}",
                item.replacen("${WorkItem}", "${WorkRecord}{WorkItem}", 1)
            )
        };
        polarion
            .check_write_owned("op", ITEM, &comment(ITEM, "1"))
            .unwrap();
        polarion
            .check_write_owned(
                "op",
                &format!("{ITEM}%3"),
                &format!("{}%4", record(ITEM, "wr-1")),
            )
            .unwrap();
        // A comment of the other target, and keys that name another object.
        for owned in [
            comment(other, "1"),
            comment(ITEM, "1${WorkItem}SB-3"),
            comment(ITEM, "1/2"),
            comment(ITEM, "1#2"),
        ] {
            assert_eq!(
                refused(polarion.check_write_owned("op", ITEM, &owned)).1,
                Some(owned)
            );
        }
    }

    #[test]
    fn guards_without_restricted_mode_are_unchanged() {
        let polarion = client(|b| b.allow_writes(["Sandbox"]));
        polarion.check_write("op", Some("Sandbox")).unwrap();
        polarion.check_unbounded_write("op").unwrap();
        polarion.check_write_target("op", ITEM).unwrap();
        polarion
            .check_field_keys("op", ["status", "resolution"])
            .unwrap();
        polarion
            .check_create("op", "Sandbox", CreateKind::WorkItem)
            .unwrap();
        polarion.check_transaction("op").unwrap();
        // Links may point to other projects; the object written to may not be in one.
        let other = "subterra:data-service:objects:/default/Prod${WorkItem}PR-1";
        polarion.check_write_targets("op", ITEM, &[other]).unwrap();
        assert_eq!(
            refused(polarion.check_write_target("op", other))
                .0
                .as_deref(),
            Some("Prod")
        );
        // Owned objects: both URIs are sent or checked.
        polarion
            .check_write_owned("op", ITEM, &format!("{ITEM}/1"))
            .unwrap();
        assert_eq!(
            refused(polarion.check_write_owned("op", ITEM, "c-1")).0,
            None
        );
    }

    #[test]
    fn messages_say_why() {
        let polarion = client(|b| b.restrict_writes([ITEM]));
        let message = |result: Result<()>| result.unwrap_err().to_string();
        assert_eq!(
            message(polarion.check_write_target("addHyperlink", "x")),
            "addHyperlink refused: `x` is not a write target of this client"
        );
        assert!(
            message(polarion.check_unbounded_write("deleteWorkItem"))
                .starts_with("deleteWorkItem refused: the client writes only to its write targets")
        );
        assert!(
            message(polarion.check_field_keys("setCustomField", ["Status"])).contains("`status`")
        );
        assert!(
            message(polarion.check_create("createWorkItem", "Sandbox", CreateKind::WorkItem))
                .ends_with("(project `Sandbox`)")
        );
        let writer = client(|b| b.allow_writes(["Sandbox"]));
        assert_eq!(
            message(writer.check_write("createWorkItem", Some("Prod"))),
            "createWorkItem refused: writes to project `Prod` are not allowed"
        );
    }
}
