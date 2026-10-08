use std::collections::BTreeSet;
use std::sync::Arc;
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
use crate::revisions::Revisions;
use crate::schema::Schema;
use crate::security::Security;
use crate::tests_mgmt::TestRuns;
use crate::wiki::Wiki;

/// Convenient access to a Polarion server, built on the SOAP web services.
///
/// Operations are grouped by area: [`Polarion::items`], [`Polarion::documents`],
/// [`Polarion::schema`], [`Polarion::projects`] and [`Polarion::users`].
///
/// **Read-only by default.** Writes must be enabled per project with
/// [`PolarionBuilder::allow_writes`]; every write method checks the target's
/// project against that list before anything is sent.
///
/// Cheap to clone; clones share the session, which is opened on the first call.
#[derive(Debug, Clone)]
pub struct Polarion {
    soap: polarion_soap::Client,
    writes: Arc<WritePolicy>,
}

#[derive(Debug, Default)]
struct WritePolicy {
    projects: BTreeSet<String>,
    global: bool,
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
    /// Calls made through it are **not** checked against the write allowlist.
    pub fn soap(&self) -> &polarion_soap::Client {
        &self.soap
    }

    /// Projects this client may write to.
    pub fn writable_projects(&self) -> impl Iterator<Item = &str> {
        self.writes.projects.iter().map(String::as_str)
    }

    /// Whether writes not tied to a project (users, global folders, ...) are allowed.
    pub fn global_writes_allowed(&self) -> bool {
        self.writes.global
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

    /// Refuse a write unless `project` is in the allowlist. Every write method calls this first.
    pub(crate) fn check_write(&self, operation: &'static str, project: Option<&str>) -> Result<()> {
        match project {
            Some(p) if self.writes.projects.contains(p) => Ok(()),
            _ => Err(Error::WriteNotAllowed {
                operation,
                project: project.map(str::to_owned),
            }),
        }
    }

    /// [`Polarion::check_write`] for the project of an object URI.
    pub(crate) fn check_write_uri(&self, operation: &'static str, uri: &str) -> Result<()> {
        self.check_write(operation, crate::uri::project_of(uri))
    }

    /// Refuse a write that is not tied to a project unless global writes are allowed.
    pub(crate) fn check_global_write(&self, operation: &'static str) -> Result<()> {
        if self.writes.global {
            Ok(())
        } else {
            Err(Error::WriteNotAllowed {
                operation,
                project: None,
            })
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

    /// Refuse unless some writes are allowed (transactions are not tied to an object).
    pub(crate) fn check_any_write(&self, operation: &'static str) -> Result<()> {
        if self.writes.global || !self.writes.projects.is_empty() {
            Ok(())
        } else {
            Err(Error::WriteNotAllowed {
                operation,
                project: None,
            })
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
        self.check_any_write("beginTransaction")?;
        Ok(self.soap.session().begin_transaction().await?)
    }

    pub async fn commit(&self) -> Result<()> {
        self.check_any_write("endTransaction")?;
        Ok(self.soap.session().end_transaction(false).await?)
    }

    pub async fn rollback(&self) -> Result<()> {
        self.check_any_write("endTransaction")?;
        Ok(self.soap.session().end_transaction(true).await?)
    }

    /// Whether an explicit transaction is open.
    pub async fn in_transaction(&self) -> Result<bool> {
        Ok(self.soap.session().transaction_exists().await?)
    }
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
    pub fn allow_global_writes(mut self, allow: bool) -> Self {
        self.writes.global = allow;
        self
    }

    pub fn build(self) -> Result<Polarion> {
        let read_only = self.writes.projects.is_empty() && !self.writes.global;
        let soap = self.soap.read_only(read_only).build()?;
        Ok(Polarion {
            soap,
            writes: Arc::new(self.writes),
        })
    }
}

pub(crate) fn not_found(kind: &'static str, id: impl std::fmt::Display) -> Error {
    Error::NotFound {
        kind,
        id: id.to_string(),
    }
}
