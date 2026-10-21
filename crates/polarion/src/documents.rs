//! Documents (called modules in the SOAP API).

use polarion_soap::services::TrackerService;
use polarion_soap::types as soap;

use crate::client::{
    CreateKind, DOCUMENT_ITEMS, DOCUMENT_MACROS, DOCUMENT_STRUCTURE, Polarion, not_found,
};
use crate::convert::{option_id, soap_custom_fields, soap_text};
use crate::error::{Error, Result};
use crate::items::baseline_and_revision;
use crate::model::{Document, WorkItem, WorkflowAction};
use crate::query::{Fields, Query, term};
use crate::restricted;
use crate::uri::{DocumentRef, ItemRef};
use crate::write::{DocumentUpdate, NewDocument, ReuseDocument};

/// The document `uri` was just created from `new` by the session of `p`: with fields to write
/// after the create, it is granted to that session ([`Polarion::grant_created`]) and they are
/// written.
async fn created(p: &Polarion, uri: String, new: &NewDocument) -> Result<DocumentRef> {
    let document = DocumentRef::from_uri(uri);
    if let Some(update) = new.follow_up() {
        p.grant_created(CreateKind::Document, &document.uri)?;
        p.documents().update(&document, &update).await?;
    }
    Ok(document)
}

/// Module fields fetched when listing documents (everything but the body).
const LIST_FIELDS: &[&str] = &[
    "id",
    "title",
    "moduleFolder",
    "moduleName",
    "type",
    "status",
    "author",
    "created",
    "updated",
    "project",
];

fn list_fields() -> Vec<String> {
    LIST_FIELDS.iter().map(|&f| f.into()).collect()
}

fn documents(modules: Vec<soap::Module>) -> Vec<Document> {
    modules
        .into_iter()
        .filter(|m| m.unresolvable != Some(true))
        .map(Document::from)
        .collect()
}

fn refs(uris: Vec<String>) -> Vec<DocumentRef> {
    uris.into_iter().map(DocumentRef::from_uri).collect()
}

/// Documents: [`Polarion::documents`].
///
/// Methods that modify data check the project of the document's URI against the write
/// allowlist, or the write targets of a restricted client, before anything is sent.
#[derive(Debug, Clone, Copy)]
pub struct Documents<'a> {
    polarion: &'a Polarion,
}

impl<'a> Documents<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn tracker(&self) -> TrackerService<'a> {
        self.polarion.soap().tracker()
    }

    /// A write to the document itself: its URI must be a write target (restricted mode), or
    /// the project of its URI writable.
    fn writable(
        &self,
        operation: &'static str,
        document: &DocumentRef,
    ) -> Result<TrackerService<'a>> {
        self.polarion.check_write_target(operation, &document.uri)?;
        Ok(self.tracker())
    }

    /// A write that may change other objects or cannot be undone: refused in restricted mode,
    /// otherwise the project of the document's URI must be writable.
    fn unbounded(
        &self,
        operation: &'static str,
        document: &DocumentRef,
    ) -> Result<TrackerService<'a>> {
        self.polarion.check_unbounded_write(operation)?;
        self.polarion.check_write_uri(operation, &document.uri)?;
        Ok(self.tracker())
    }

    // ---------------------------------------------------------------- reads

    /// Document space ids of a project (`_default` is the default space).
    pub async fn spaces(&self, project: &str) -> Result<Vec<String>> {
        Ok(self.tracker().get_document_spaces(project).await?)
    }

    /// Sub-folders of a location relative to the project's documents folder.
    pub async fn sub_folders(&self, project: &str, location: &str) -> Result<Vec<String>> {
        Ok(self
            .tracker()
            .get_modules_sub_folders(project, location)
            .await?)
    }

    /// Documents of a project, or of one space, sorted by space and name. Bodies are not fetched.
    pub async fn list(&self, project: &str, space: Option<&str>) -> Result<Vec<Document>> {
        let fields = list_fields();
        let tracker = self.tracker();
        let modules = match space {
            Some(space) => {
                tracker
                    .get_modules_with_fields(project, space, &fields)
                    .await?
            }
            None => {
                let query = format!("project.id:{}", term(project));
                tracker
                    .query_modules(Some(&query), "title", &fields, -1)
                    .await?
            }
        };
        let mut docs = documents(modules);
        docs.sort_by(|a, b| (&a.space, &a.name).cmp(&(&b.space, &b.name)));
        Ok(docs)
    }

    /// The documents in one space with all fields, including their bodies.
    pub async fn list_full(&self, project: &str, space: &str) -> Result<Vec<Document>> {
        Ok(documents(self.tracker().get_modules(project, space).await?))
    }

    /// References of the documents in one space.
    pub async fn refs(&self, project: &str, space: &str) -> Result<Vec<DocumentRef>> {
        Ok(refs(self.tracker().get_module_uris(project, space).await?))
    }

    /// One document. With `with_body`, all fields including the body (which can be large);
    /// otherwise the same fields as [`Documents::list`].
    pub async fn get(
        &self,
        project: &str,
        space: &str,
        name: &str,
        with_body: bool,
    ) -> Result<Document> {
        let location = format!("{space}/{name}");
        let tracker = self.tracker();
        let m = if with_body {
            tracker.get_module_by_location(project, &location).await?
        } else {
            tracker
                .get_module_by_location_with_fields(project, &location, &list_fields())
                .await?
        };
        found(m, format!("{project}/{location}"))
    }

    /// A document by reference: all fields (`fields` `None`) or only `fields`.
    pub async fn get_by_ref(
        &self,
        document: &DocumentRef,
        fields: Option<&Fields>,
    ) -> Result<Document> {
        let tracker = self.tracker();
        let m = match fields {
            None => tracker.get_module_by_uri(&document.uri).await?,
            Some(fields) => {
                tracker
                    .get_module_by_uri_with_fields(&document.uri, fields.keys())
                    .await?
            }
        };
        found(m, document)
    }

    /// Documents matching a Lucene query, optionally in a baseline. Without
    /// [`Query::fields`] the fields of [`Documents::list`] are fetched (no bodies).
    pub async fn query(&self, query: &Query) -> Result<Vec<Document>> {
        if query.revision_value().is_some() {
            return Err(baseline_and_revision());
        }
        let default = list_fields();
        let fields = query.field_list().map_or(default.as_slice(), Fields::keys);
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        let tracker = self.tracker();
        let modules = match query.baseline_revision() {
            None => {
                tracker
                    .query_modules(q.as_deref(), sort, fields, limit)
                    .await?
            }
            Some(b) => {
                tracker
                    .query_modules_in_baseline(q.as_deref(), sort, b, fields, limit)
                    .await?
            }
        };
        Ok(documents(modules))
    }

    /// References of the documents matching a Lucene query, optionally in a baseline.
    pub async fn query_refs(&self, query: &Query) -> Result<Vec<DocumentRef>> {
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        let tracker = self.tracker();
        let uris = match query.baseline_revision() {
            None => tracker.query_module_uris(q.as_deref(), sort, limit).await?,
            Some(b) => {
                tracker
                    .query_module_uris_in_baseline(q.as_deref(), sort, b, limit)
                    .await?
            }
        };
        Ok(refs(uris))
    }

    /// Documents selected by SQL (`SELECT ... FROM MODULE ...`), optionally in a baseline.
    pub async fn query_sql(
        &self,
        sql: &str,
        fields: Option<&Fields>,
        baseline: Option<&str>,
    ) -> Result<Vec<Document>> {
        let default = list_fields();
        let fields = fields.map_or(default.as_slice(), Fields::keys);
        let tracker = self.tracker();
        let modules = match baseline {
            None => tracker.query_modules_by_sql(sql, fields).await?,
            Some(b) => {
                tracker
                    .query_modules_in_baseline_by_sql(sql, b, fields)
                    .await?
            }
        };
        Ok(documents(modules))
    }

    /// References of the documents selected by SQL, optionally in a baseline.
    pub async fn query_refs_sql(
        &self,
        sql: &str,
        baseline: Option<&str>,
    ) -> Result<Vec<DocumentRef>> {
        let tracker = self.tracker();
        let uris = match baseline {
            None => tracker.query_module_uris_by_sql(sql).await?,
            Some(b) => tracker.query_module_uris_in_baseline_by_sql(sql, b).await?,
        };
        Ok(refs(uris))
    }

    /// The work items of a document, in document order.
    pub async fn work_items(
        &self,
        document: &DocumentRef,
        fields: &Fields,
    ) -> Result<Vec<WorkItem>> {
        self.work_items_under(document, None, true, fields).await
    }

    /// The work items under `parent` (or the top level), the whole subtree if `deep`.
    pub async fn work_items_under(
        &self,
        document: &DocumentRef,
        parent: Option<&ItemRef>,
        deep: bool,
        fields: &Fields,
    ) -> Result<Vec<WorkItem>> {
        let items = self
            .tracker()
            .get_module_work_items(
                &document.uri,
                parent.map(|p| p.uri.as_str()),
                deep,
                fields.keys(),
            )
            .await?;
        Ok(items.into_iter().map(WorkItem::from).collect())
    }

    /// References of the work items under `parent` (or the top level), the whole subtree if `deep`.
    pub async fn work_item_refs(
        &self,
        document: &DocumentRef,
        parent: Option<&ItemRef>,
        deep: bool,
    ) -> Result<Vec<ItemRef>> {
        let uris = self
            .tracker()
            .get_module_work_item_uris(&document.uri, parent.map(|p| p.uri.as_str()), deep)
            .await?;
        Ok(uris.into_iter().map(ItemRef::from_uri).collect())
    }

    /// Workflow actions that can be performed on the document now.
    pub async fn actions(&self, document: &DocumentRef) -> Result<Vec<WorkflowAction>> {
        let actions = self.tracker().get_available_actions(&document.uri).await?;
        Ok(actions.into_iter().map(WorkflowAction::from).collect())
    }

    /// Export the document as PDF.
    pub async fn export_pdf(
        &self,
        document: &DocumentRef,
        options: &soap::PdfProperties,
    ) -> Result<Vec<u8>> {
        Ok(self
            .tracker()
            .export_document_to_pdf(&document.uri, options)
            .await?)
    }

    // --------------------------------------------------------------- writes

    /// Create a live document; returns its reference. A client restricted to write targets
    /// never creates one with work item macros in its body.
    ///
    /// With a type, status or custom fields ([`NewDocument`]), the create and the `updateModule`
    /// that writes them run as one transaction in a Polarion session of its own (all or nothing).
    pub async fn create(&self, new: &NewDocument) -> Result<DocumentRef> {
        if self.polarion.writes_restricted()
            && new
                .body
                .as_deref()
                .is_some_and(restricted::has_work_item_macro)
        {
            return Err(Polarion::refused(
                "createDocument",
                None,
                None,
                Some(DOCUMENT_MACROS),
            ));
        }
        // Before the budget is taken: a refused create takes nothing.
        self.check_follow_up("createDocument", new)?;
        self.polarion
            .check_create("createDocument", &new.project, CreateKind::Document)?;
        let new = new.clone();
        let writes = 1 + usize::from(new.follow_up().is_some());
        self.polarion
            .atomic("createDocument", writes, move |p| async move {
                let types: Vec<soap::EnumOptionId> =
                    new.allowed_types.iter().map(|t| option_id(t)).collect();
                // The guard ran above: the request is sent as it is.
                let uri = p
                    .soap()
                    .tracker()
                    .create_document(
                        &new.project,
                        new.space.as_deref(),
                        &new.name,
                        new.title.as_deref(),
                        &types,
                        &option_id(&new.structure_link_role),
                        new.body.as_deref(),
                    )
                    .await?;
                created(&p, uri, &new).await
            })
            .await
    }

    /// Create an old-style module (`createModule`); `parent_to_child` sets the link direction
    /// of the structure. Prefer [`Documents::create`]. Type, status and custom fields as there;
    /// never in restricted mode.
    pub async fn create_module(
        &self,
        new: &NewDocument,
        parent_to_child: bool,
    ) -> Result<DocumentRef> {
        self.check_follow_up("createModule", new)?;
        self.polarion.check_unbounded_write("createModule")?;
        self.polarion
            .check_write("createModule", Some(&new.project))?;
        let new = new.clone();
        let writes = 1 + usize::from(new.follow_up().is_some());
        self.polarion
            .atomic("createModule", writes, move |p| async move {
                let types: Vec<soap::EnumOptionId> =
                    new.allowed_types.iter().map(|t| option_id(t)).collect();
                let uri = p
                    .soap()
                    .tracker()
                    .create_module(
                        &new.project,
                        new.space.as_deref(),
                        &new.name,
                        &types,
                        &option_id(&new.structure_link_role),
                        parent_to_child,
                        new.body.as_deref(),
                    )
                    .await?;
                created(&p, uri, &new).await
            })
            .await
    }

    /// The checks of the fields written after a create, before anything is sent: the custom
    /// field keys and the status (refused in restricted mode, as for [`Documents::update`]).
    fn check_follow_up(&self, operation: &'static str, new: &NewDocument) -> Result<()> {
        let keys = new
            .custom_fields
            .keys()
            .map(String::as_str)
            .chain(new.status.as_ref().map(|_| "status"));
        self.polarion.check_field_keys(operation, keys)
    }

    /// Change fields of a document.
    ///
    /// A client restricted to write targets never changes the structure link role, never writes
    /// a body with work item macros, and writes the body and outline numbering only of a document
    /// that holds no work items (it reads them first): those writes change the document's work
    /// items, which are not the document.
    pub async fn update(&self, document: &DocumentRef, update: &DocumentUpdate) -> Result<()> {
        let tracker = self.writable("updateModule", document)?;
        let keys = update
            .custom_fields
            .keys()
            .map(String::as_str)
            .chain(update.status.as_ref().map(|_| "status"));
        self.polarion.check_field_keys("updateModule", keys)?;
        if self.polarion.writes_restricted() {
            let refused = |reason| Polarion::refused("updateModule", None, None, Some(reason));
            if update.structure_link_role.is_some() {
                return Err(refused(DOCUMENT_STRUCTURE));
            }
            if update
                .body
                .as_ref()
                .is_some_and(|b| restricted::has_work_item_macro(&b.content))
            {
                return Err(refused(DOCUMENT_MACROS));
            }
            if update.body.is_some() || update.uses_outline_numbering.is_some() {
                let items = tracker
                    .get_module_work_item_uris(&document.uri, None, true)
                    .await?;
                if !items.is_empty() {
                    return Err(refused(DOCUMENT_ITEMS));
                }
                self.polarion.approve_document_write(&document.uri);
            }
        }
        let module = soap::Module {
            uri: Some(document.uri.clone()),
            title: update.title.clone(),
            type_: update.kind.as_deref().map(option_id),
            status: update.status.as_deref().map(option_id),
            home_page_content: update.body.as_ref().map(soap_text),
            structure_link_role: update.structure_link_role.as_deref().map(option_id),
            uses_outline_numbering: update.uses_outline_numbering,
            custom_fields: soap_custom_fields(&update.custom_fields)?,
            ..soap::Module::default()
        };
        Ok(tracker.update_module(&module).await?)
    }

    pub async fn delete(&self, document: &DocumentRef) -> Result<()> {
        let tracker = self.unbounded("deleteModule", document)?;
        Ok(tracker.delete_module(&document.uri).await?)
    }

    /// Set the first title heading in the content; returns the heading's URI if one was updated.
    pub async fn update_title_heading(
        &self,
        document: &DocumentRef,
        title: Option<&str>,
    ) -> Result<Option<String>> {
        let tracker = self.unbounded("updateTitleHeadingInDocument", document)?;
        let uri = tracker
            .update_title_heading_in_document(&document.uri, title)
            .await?;
        Ok(Some(uri).filter(|u| !u.is_empty()))
    }

    /// Create a new document from an existing one (reuse); returns the new document.
    pub async fn reuse(&self, source: &DocumentRef, target: &ReuseDocument) -> Result<DocumentRef> {
        self.polarion.check_unbounded_write("reuseDocument")?;
        self.polarion
            .check_write("reuseDocument", Some(&target.project))?;
        let role = target.link_role.as_deref().map(option_id);
        let uri = self
            .tracker()
            .reuse_document(
                &source.uri,
                &target.project,
                target.space.as_deref(),
                &target.name,
                target.title.as_deref(),
                target.update_title_in_document,
                role.as_ref(),
                &target.derived_fields,
            )
            .await?;
        Ok(DocumentRef::from_uri(uri))
    }

    /// Old-style module reuse (`reuseModule`), copying `fields` except `except_fields`.
    /// Prefer [`Documents::reuse`].
    pub async fn reuse_module(
        &self,
        source: &DocumentRef,
        target: &ReuseDocument,
        fields: &[String],
        except_fields: &[String],
    ) -> Result<DocumentRef> {
        self.polarion.check_unbounded_write("reuseModule")?;
        self.polarion
            .check_write("reuseModule", Some(&target.project))?;
        let role = target.link_role.as_deref().map(option_id);
        let uri = self
            .tracker()
            .reuse_module(
                &source.uri,
                &target.project,
                target.space.as_deref().unwrap_or("_default"),
                &target.name,
                role.as_ref(),
                fields,
                except_fields,
                &target.derived_fields,
            )
            .await?;
        Ok(DocumentRef::from_uri(uri))
    }

    /// Update a derived (reused) document to a revision of its source (`None`: HEAD).
    pub async fn update_derived(
        &self,
        document: &DocumentRef,
        revision: Option<&str>,
        auto_suspect: bool,
    ) -> Result<()> {
        let tracker = self.unbounded("updateDerivedDocument", document)?;
        Ok(tracker
            .update_derived_document(&document.uri, revision, auto_suspect)
            .await?)
    }

    /// Old-style variant of [`Documents::update_derived`] (`updateDerivedModule`).
    pub async fn update_derived_module(
        &self,
        document: &DocumentRef,
        revision: Option<&str>,
    ) -> Result<()> {
        let tracker = self.unbounded("updateDerivedModule", document)?;
        Ok(tracker
            .update_derived_module(&document.uri, revision)
            .await?)
    }

    /// Comment on the document, optionally referring to one of its work items; returns the
    /// comment's URI. A client restricted to write targets refuses a comment that refers to a
    /// work item.
    pub async fn add_comment(
        &self,
        document: &DocumentRef,
        about: Option<&ItemRef>,
        text: &crate::model::Text,
    ) -> Result<String> {
        let text = soap_text(text);
        Ok(match about {
            None => {
                self.writable("addComment", document)?
                    .add_comment(&document.uri, None, Some(&text))
                    .await?
            }
            Some(item) => {
                self.unbounded("createDocumentCommentReferringWI", document)?
                    .create_document_comment_referring_wi(&document.uri, &item.uri, &text)
                    .await?
            }
        })
    }

    /// Reply to a comment of the document; returns the reply's URI.
    pub async fn reply_to_comment(
        &self,
        document: &DocumentRef,
        comment_uri: &str,
        text: &crate::model::Text,
    ) -> Result<String> {
        self.polarion
            .check_write_owned("addComment", &document.uri, comment_uri)?;
        Ok(self
            .tracker()
            .add_comment(comment_uri, None, Some(&soap_text(text)))
            .await?)
    }

    /// Perform a workflow action on the document (see [`Documents::actions`]).
    pub async fn perform_action(&self, document: &DocumentRef, action_id: i32) -> Result<()> {
        let tracker = self.unbounded("performWorkflowAction", document)?;
        Ok(tracker
            .perform_workflow_action(&document.uri, action_id)
            .await?)
    }

    /// Move the document to `status` with the available workflow action leading there.
    pub async fn transition(&self, document: &DocumentRef, status: &str) -> Result<WorkflowAction> {
        self.unbounded("performWorkflowAction", document)?;
        let action = self
            .actions(document)
            .await?
            .into_iter()
            .find(|a| a.target_status.as_deref() == Some(status))
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no available workflow action moves {document} to status `{status}`"
                ))
            })?;
        self.perform_action(document, action.id).await?;
        Ok(action)
    }
}

fn found(m: soap::Module, id: impl std::fmt::Display) -> Result<Document> {
    if m.unresolvable == Some(true) || (m.uri.is_none() && m.title.is_none()) {
        return Err(not_found("document", id));
    }
    Ok(m.into())
}
