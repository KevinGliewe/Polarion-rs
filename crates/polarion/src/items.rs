//! Work items: reads and (guarded) writes.

use polarion_soap::services::TrackerService;
use polarion_soap::types as soap;

use crate::client::{CreateKind, Polarion, not_found};
use crate::convert::{self, any_value, option_id, soap_text};
use crate::error::{Error, Result};
use crate::model::{Change, EnumOption, FieldValue, Link, Text, User, WorkItem, WorkflowAction};
use crate::query::{Fields, WorkItemQuery};
use crate::uri::{DocumentRef, ItemRef};
use crate::write::{NewWorkItem, NewWorkRecord, WorkItemUpdate};

/// Work items: [`Polarion::items`].
///
/// Work items are addressed by [`ItemRef`]: `ItemRef::new("PROJ", "PROJ-1")`, or a
/// reference taken from a result ([`WorkItem::reference`], [`Link::item`]).
///
/// Every method that modifies data first checks the project of the URI it sends against the
/// write allowlist ([`crate::PolarionBuilder::allow_writes`]), or the write targets of a
/// restricted client ([`crate::PolarionBuilder::restrict_writes`]), and fails with
/// [`Error::WriteNotAllowed`] without sending anything.
#[derive(Debug, Clone, Copy)]
pub struct WorkItems<'a> {
    polarion: &'a Polarion,
}

fn items(list: Vec<soap::WorkItem>) -> Vec<WorkItem> {
    list.into_iter()
        .filter(|w| w.unresolvable != Some(true))
        .map(WorkItem::from)
        .collect()
}

pub(crate) fn baseline_and_revision() -> Error {
    Error::Invalid("a query can use a baseline or a revision, not both".into())
}

fn found(item: soap::WorkItem, reference: &ItemRef) -> Result<WorkItem> {
    if item.unresolvable == Some(true) || (item.id.is_none() && item.uri.is_none()) {
        return Err(not_found("work item", reference));
    }
    Ok(item.into())
}

impl<'a> WorkItems<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn tracker(&self) -> TrackerService<'a> {
        self.polarion.soap().tracker()
    }

    /// A write to the work item itself: its URI must be a write target (restricted mode), or
    /// the project of its URI writable.
    pub(crate) fn writable(
        &self,
        operation: &'static str,
        item: &ItemRef,
    ) -> Result<TrackerService<'a>> {
        self.polarion.check_write_target(operation, &item.uri)?;
        Ok(self.tracker())
    }

    /// A write that may change other objects or cannot be undone: refused in restricted mode,
    /// otherwise the project of the item's URI must be writable.
    fn unbounded(&self, operation: &'static str, item: &ItemRef) -> Result<TrackerService<'a>> {
        self.polarion.check_unbounded_write(operation)?;
        self.polarion.check_write_uri(operation, &item.uri)?;
        Ok(self.tracker())
    }

    // ---------------------------------------------------------------- reads

    /// A work item with all fields.
    pub async fn get(&self, item: &ItemRef) -> Result<WorkItem> {
        let tracker = self.tracker();
        // By id where possible: it does not depend on the URI format.
        let w = match (&item.project, &item.id) {
            (Some(project), Some(id)) => tracker.get_work_item_by_id(project, id).await?,
            _ => tracker.get_work_item_by_uri(&item.uri).await?,
        };
        found(w, item)
    }

    /// A work item with only the given fields filled in.
    pub async fn get_fields(&self, item: &ItemRef, fields: &Fields) -> Result<WorkItem> {
        let tracker = self.tracker();
        let w = match (&item.project, &item.id) {
            (Some(project), Some(id)) => {
                tracker
                    .get_work_item_by_ids_with_fields(project, id, fields.keys())
                    .await?
            }
            _ => {
                tracker
                    .get_work_item_by_uri_with_fields(&item.uri, fields.keys())
                    .await?
            }
        };
        found(w, item)
    }

    /// A work item as it was in a repository revision (all fields, or only `fields`).
    pub async fn get_in_revision(
        &self,
        item: &ItemRef,
        revision: &str,
        fields: Option<&Fields>,
    ) -> Result<WorkItem> {
        let tracker = self.tracker();
        let w = match fields {
            Some(fields) => {
                tracker
                    .get_work_item_by_uri_in_revision_with_fields(
                        &item.uri,
                        revision,
                        fields.keys(),
                    )
                    .await?
            }
            None => {
                tracker
                    .get_work_item_by_uri_in_revision(&item.uri, revision)
                    .await?
            }
        };
        found(w, item)
    }

    /// Work items matching a Lucene query, optionally in a baseline or as of a revision.
    pub async fn query(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let t = self.tracker();
        let (q, sort) = (query.lucene(), query.sort_key());
        let standard = Fields::standard();
        let fields = query.field_list().unwrap_or(&standard).keys();
        let list = match (
            query.baseline_revision(),
            query.revision_value(),
            query.limit_value(),
        ) {
            (Some(_), Some(_), _) => return Err(baseline_and_revision()),
            (None, None, Some(_)) => {
                t.query_work_items_limited(&q, sort, fields, query.results_limit())
                    .await?
            }
            (None, None, None) => t.query_work_items(&q, sort, fields).await?,
            (Some(b), None, Some(_)) => {
                t.query_work_items_in_baseline_limited(&q, sort, b, fields, query.results_limit())
                    .await?
            }
            (Some(b), None, None) => t.query_work_items_in_baseline(&q, sort, b, fields).await?,
            (None, Some(r), Some(_)) => {
                t.query_work_items_in_revision_limited(&q, sort, r, fields, query.results_limit())
                    .await?
            }
            (None, Some(r), None) => t.query_work_items_in_revision(&q, sort, r, fields).await?,
        };
        Ok(items(list))
    }

    /// References of the work items matching a query; cheaper than [`WorkItems::query`].
    /// Fields and revision are ignored.
    pub async fn query_refs(&self, query: &WorkItemQuery) -> Result<Vec<ItemRef>> {
        let t = self.tracker();
        let (q, sort) = (query.lucene(), query.sort_key());
        let uris = match (query.baseline_revision(), query.limit_value()) {
            (None, Some(_)) => {
                t.query_work_item_uris_limited(&q, sort, query.results_limit())
                    .await?
            }
            (None, None) => t.query_work_item_uris(&q, sort).await?,
            (Some(b), Some(_)) => {
                t.query_work_item_uris_in_baseline_limited(&q, sort, b, query.results_limit())
                    .await?
            }
            (Some(b), None) => t.query_work_item_uris_in_baseline(&q, sort, b).await?,
        };
        Ok(uris.into_iter().map(ItemRef::from_uri).collect())
    }

    /// Work items selected by an SQL query against Polarion's database
    /// (`SELECT ... FROM WORKITEM ...`), optionally in a baseline.
    pub async fn query_sql(
        &self,
        sql: &str,
        fields: &Fields,
        baseline: Option<&str>,
    ) -> Result<Vec<WorkItem>> {
        let t = self.tracker();
        let list = match baseline {
            None => t.query_work_items_by_sql(sql, fields.keys()).await?,
            Some(b) => {
                t.query_work_items_in_baseline_by_sql(sql, b, fields.keys())
                    .await?
            }
        };
        Ok(items(list))
    }

    /// References of the work items selected by an SQL query, optionally in a baseline.
    pub async fn query_refs_sql(&self, sql: &str, baseline: Option<&str>) -> Result<Vec<ItemRef>> {
        let t = self.tracker();
        let uris = match baseline {
            None => t.query_work_item_uris_by_sql(sql).await?,
            Some(b) => t.query_work_item_uris_in_baseline_by_sql(sql, b).await?,
        };
        Ok(uris.into_iter().map(ItemRef::from_uri).collect())
    }

    /// Number of work items matching the query (sort, limit, fields and baseline are ignored).
    pub async fn count(&self, query: &WorkItemQuery) -> Result<u32> {
        let count = self.tracker().get_work_items_count(&query.lucene()).await?;
        Ok(u32::try_from(count).unwrap_or(0))
    }

    /// Links from other work items to this one; each link's `item` is the source.
    pub async fn back_links(&self, item: &ItemRef) -> Result<Vec<Link>> {
        let links = self.tracker().get_back_linked_workitems(&item.uri).await?;
        Ok(links.into_iter().filter_map(convert::link).collect())
    }

    /// The history of a work item, oldest first.
    pub async fn history(&self, item: &ItemRef) -> Result<Vec<Change>> {
        let changes = self.tracker().generate_history(&item.uri, &[], &[]).await?;
        Ok(changes.into_iter().map(Change::from).collect())
    }

    /// Workflow actions that can be performed now.
    pub async fn actions(&self, item: &ItemRef) -> Result<Vec<WorkflowAction>> {
        let actions = self.tracker().get_available_actions(&item.uri).await?;
        Ok(actions.into_iter().map(WorkflowAction::from).collect())
    }

    /// Workflow actions that exist but cannot be performed now, with the reason in `message`.
    pub async fn unavailable_actions(&self, item: &ItemRef) -> Result<Vec<WorkflowAction>> {
        let actions = self.tracker().get_unavailable_actions(&item.uri).await?;
        Ok(actions.into_iter().map(WorkflowAction::from).collect())
    }

    /// Users that may be assigned to the work item.
    pub async fn allowed_assignees(&self, item: &ItemRef) -> Result<Vec<User>> {
        let users = self.tracker().get_allowed_assignees(&item.uri).await?;
        Ok(users.into_iter().map(User::from).collect())
    }

    /// Users that may approve the work item.
    pub async fn allowed_approvers(&self, item: &ItemRef) -> Result<Vec<User>> {
        let users = self.tracker().get_allowed_approvers(&item.uri).await?;
        Ok(users.into_iter().map(User::from).collect())
    }

    /// The options a field of this work item can take (respecting dependencies on other fields).
    pub async fn field_options(&self, item: &ItemRef, key: &str) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_enum_options_for_key(&item.uri, key)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// The content of an attachment.
    pub async fn attachment(&self, item: &ItemRef, attachment_id: &str) -> Result<Vec<u8>> {
        Ok(self
            .tracker()
            .get_attachment(&item.uri, attachment_id)
            .await?)
    }

    /// The title in a language (`None`: the default language).
    pub async fn localized_title(&self, item: &ItemRef, language: Option<&str>) -> Result<String> {
        Ok(self
            .tracker()
            .get_localized_work_item_title(&item.uri, language)
            .await?)
    }

    /// The description in a language (`None`: the default language).
    pub async fn localized_description(
        &self,
        item: &ItemRef,
        language: Option<&str>,
    ) -> Result<Text> {
        let text = self
            .tracker()
            .get_localized_work_item_description(&item.uri, language)
            .await?;
        Ok(text.into())
    }

    /// The value of one custom field (`None` if empty).
    pub async fn custom_field(&self, item: &ItemRef, key: &str) -> Result<Option<FieldValue>> {
        let field = self.tracker().get_custom_field(&item.uri, key).await?;
        Ok(field.value.and_then(convert::field_value))
    }

    /// The keys of the custom fields defined for this work item.
    pub async fn custom_field_keys(&self, item: &ItemRef) -> Result<Vec<String>> {
        Ok(self.tracker().get_custom_field_keys(&item.uri).await?)
    }

    /// The options of an enumeration (by enum id) that this work item can use.
    pub async fn enum_options(&self, item: &ItemRef, enum_id: &str) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_enum_options_for_id(&item.uri, enum_id)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// Ids of the options of a field that are available now (respecting the workflow and
    /// dependencies).
    pub async fn available_option_ids(&self, item: &ItemRef, key: &str) -> Result<Vec<String>> {
        let ids = self
            .tracker()
            .get_available_enum_option_ids_for_key(&item.uri, key)
            .await?;
        Ok(ids.into_iter().filter_map(|o| o.id).collect())
    }

    /// Ids of the options of an enumeration (by enum id) that are available now.
    pub async fn available_enum_option_ids(
        &self,
        item: &ItemRef,
        enum_id: &str,
    ) -> Result<Vec<String>> {
        let ids = self
            .tracker()
            .get_available_enum_option_ids_for_id(&item.uri, enum_id)
            .await?;
        Ok(ids.into_iter().filter_map(|o| o.id).collect())
    }

    /// One option of a field, by option id.
    pub async fn field_option(
        &self,
        item: &ItemRef,
        key: &str,
        option: &str,
    ) -> Result<EnumOption> {
        let o = self
            .tracker()
            .get_enum_option_with_key(&item.uri, key, &option_id(option))
            .await?;
        Ok(o.into())
    }

    /// One option of an enumeration (by enum id), by option id.
    pub async fn enum_option(
        &self,
        item: &ItemRef,
        enum_id: &str,
        option: &str,
    ) -> Result<EnumOption> {
        let o = self
            .tracker()
            .get_enum_option_with_enum_id(&item.uri, enum_id, &option_id(option))
            .await?;
        Ok(o.into())
    }

    /// The options of a field, filtered by the option of the field it depends on (`filter`).
    pub async fn filtered_field_options(
        &self,
        item: &ItemRef,
        key: &str,
        filter: Option<&str>,
    ) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_filtered_enum_options_for_key(&item.uri, key, filter)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// The workflow action performed when the work item is created, if any.
    pub async fn initial_action(&self, item: &ItemRef) -> Result<Option<WorkflowAction>> {
        let a = self
            .tracker()
            .get_initial_workflow_action(&item.uri)
            .await?;
        Ok((a.action_name.is_some() || a.native_action_id.is_some()).then(|| a.into()))
    }

    /// Whether a comment is in a resolved comment thread.
    pub async fn is_comment_resolved(&self, comment_uri: &str) -> Result<bool> {
        Ok(self.tracker().is_resolved_comment(comment_uri).await?)
    }

    /// Source code resources linked to the work item.
    pub async fn linked_resources(&self, item: &ItemRef) -> Result<Vec<soap::LinkedResource>> {
        Ok(self.tracker().get_linked_resources(&item.uri).await?)
    }

    /// Repository revisions in which the work item changed (newest last).
    pub async fn revisions(&self, item: &ItemRef) -> Result<Vec<String>> {
        Ok(self.tracker().get_revisions(&item.uri).await?)
    }

    // --------------------------------------------------------------- writes

    /// Create a work item; returns its reference.
    pub async fn create(&self, new: &NewWorkItem) -> Result<ItemRef> {
        // Before the budget is taken: the SOAP write filter would refuse the request.
        self.polarion.check_field_keys(
            "createWorkItem",
            new.custom_fields.keys().map(String::as_str),
        )?;
        self.polarion
            .check_create("createWorkItem", &new.project, CreateKind::WorkItem)?;
        let content = self.content(new).await?;
        let uri = self.tracker().create_work_item(&content).await?;
        Ok(ItemRef::from_uri(uri))
    }

    /// Create a work item inside a document, under `parent` or at the top level.
    pub async fn create_in_document(
        &self,
        new: &NewWorkItem,
        document: &DocumentRef,
        parent: Option<&ItemRef>,
    ) -> Result<ItemRef> {
        self.polarion
            .check_unbounded_write("createWorkItemInModule")?;
        self.polarion
            .check_write("createWorkItemInModule", Some(&new.project))?;
        self.polarion
            .check_write_uri("createWorkItemInModule", &document.uri)?;
        let content = self.content(new).await?;
        let uri = self
            .tracker()
            .create_work_item_in_module(&document.uri, parent.map(|p| p.uri.as_str()), &content)
            .await?;
        Ok(ItemRef::from_uri(uri))
    }

    /// `createWorkItem` content; the project is looked up so its URI is exact.
    async fn content(&self, new: &NewWorkItem) -> Result<soap::WorkItem> {
        let project = self
            .polarion
            .soap()
            .project()
            .get_project(&new.project)
            .await?;
        if project.unresolvable == Some(true) || project.uri.is_none() {
            return Err(not_found("project", &new.project));
        }
        convert::new_work_item(new, project)
    }

    /// Change fields. Standard fields go in one `updateWorkItem`, each custom field
    /// through `setCustomField`, and cleared fields through one `setFieldsNull`.
    pub async fn update(&self, item: &ItemRef, update: &WorkItemUpdate) -> Result<()> {
        let tracker = self.writable("updateWorkItem", item)?;
        let keys = update
            .custom_fields
            .keys()
            .chain(&update.clear)
            .map(String::as_str)
            .chain(update.resolution.as_ref().map(|_| "resolution"));
        self.polarion.check_field_keys("updateWorkItem", keys)?;
        // Convert everything before sending anything.
        let custom: Vec<soap::CustomField> = update
            .custom_fields
            .iter()
            .filter_map(|(key, value)| Some((key, value.as_ref()?)))
            .map(|(key, value)| {
                Ok(soap::CustomField {
                    key: key.clone(),
                    parent_item_uri: item.uri.clone(),
                    value: Some(any_value(value)?),
                })
            })
            .collect::<Result<_>>()?;
        let clear: Vec<String> = update
            .custom_fields
            .iter()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.clone())
            .chain(update.clear.iter().cloned())
            .collect();

        if update.has_standard_fields() {
            tracker
                .update_work_item(&convert::work_item_update(&item.uri, update))
                .await?;
        }
        for field in &custom {
            tracker.set_custom_field(field).await?;
        }
        if !clear.is_empty() {
            tracker.set_fields_null(&item.uri, &clear).await?;
        }
        Ok(())
    }

    /// Set (`Some`) or clear (`None`) one custom field.
    pub async fn set_custom_field(
        &self,
        item: &ItemRef,
        key: &str,
        value: Option<&FieldValue>,
    ) -> Result<()> {
        let tracker = self.writable("setCustomField", item)?;
        self.polarion.check_field_keys("setCustomField", [key])?;
        match value {
            Some(value) => {
                let field = soap::CustomField {
                    key: key.to_owned(),
                    parent_item_uri: item.uri.clone(),
                    value: Some(any_value(value)?),
                };
                tracker.set_custom_field(&field).await?;
            }
            None => {
                tracker
                    .set_fields_null(&item.uri, &[key.to_owned()])
                    .await?
            }
        }
        Ok(())
    }

    /// Clear fields (standard or custom) by key.
    pub async fn clear_fields(&self, item: &ItemRef, keys: &[String]) -> Result<()> {
        let tracker = self.writable("setFieldsNull", item)?;
        self.polarion
            .check_field_keys("setFieldsNull", keys.iter().map(String::as_str))?;
        Ok(tracker.set_fields_null(&item.uri, keys).await?)
    }

    pub async fn delete(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.unbounded("deleteWorkItem", item)?;
        Ok(tracker.delete_work_item(&item.uri).await?)
    }

    /// Move the work item into a document (or within it), under `parent` or at the top level.
    /// `position` counts among the parent's children; `None` appends.
    pub async fn move_to_document(
        &self,
        item: &ItemRef,
        document: &DocumentRef,
        parent: Option<&ItemRef>,
        position: Option<u32>,
        retain_document_flow: bool,
    ) -> Result<()> {
        let tracker = self.unbounded("moveWorkItemToDocument", item)?;
        self.polarion
            .check_write_uri("moveWorkItemToDocument", &document.uri)?;
        let position = position.map_or(-1, |p| i32::try_from(p).unwrap_or(i32::MAX));
        Ok(tracker
            .move_work_item_to_document(
                &item.uri,
                &document.uri,
                parent.map(|p| p.uri.as_str()),
                position,
                retain_document_flow,
            )
            .await?)
    }

    // ---- workflow

    /// Perform a workflow action by its id (see [`WorkItems::actions`]).
    pub async fn perform_action(&self, item: &ItemRef, action_id: i32) -> Result<()> {
        let tracker = self.unbounded("performWorkflowAction", item)?;
        Ok(tracker
            .perform_workflow_action(&item.uri, action_id)
            .await?)
    }

    /// Move the work item to `status` with the available workflow action leading there.
    /// Returns the action performed; [`Error::Invalid`] if no available action leads to `status`.
    pub async fn transition(&self, item: &ItemRef, status: &str) -> Result<WorkflowAction> {
        self.unbounded("performWorkflowAction", item)?;
        let actions = self.actions(item).await?;
        let action = actions
            .into_iter()
            .find(|a| a.target_status.as_deref() == Some(status))
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no available workflow action moves {item} to status `{status}`"
                ))
            })?;
        self.perform_action(item, action.id).await?;
        Ok(action)
    }

    /// Reset the workflow: perform the initial action and set the initial status.
    pub async fn reset_workflow(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.unbounded("resetWorkflow", item)?;
        Ok(tracker.reset_workflow(&item.uri).await?)
    }

    // ---- comments

    /// Add a comment; returns its URI.
    pub async fn add_comment(
        &self,
        item: &ItemRef,
        title: Option<&str>,
        text: &Text,
    ) -> Result<String> {
        let tracker = self.writable("addComment", item)?;
        Ok(tracker
            .add_comment(&item.uri, title, Some(&soap_text(text)))
            .await?)
    }

    /// Add a comment visible only to the given users or roles; returns its URI.
    /// (Uses Polarion's deprecated `createCommentNew`, the only operation with visibility.)
    pub async fn add_restricted_comment(
        &self,
        item: &ItemRef,
        title: &str,
        text: &Text,
        visible_to: &[String],
    ) -> Result<String> {
        let tracker = self.writable("createCommentNew", item)?;
        #[allow(deprecated)]
        let uri = tracker
            .create_comment_new(&item.uri, title, &soap_text(text), visible_to)
            .await?;
        Ok(uri)
    }

    /// Reply to a comment of `item`; returns the reply's URI.
    pub async fn reply_to_comment(
        &self,
        item: &ItemRef,
        comment_uri: &str,
        text: &Text,
    ) -> Result<String> {
        self.polarion
            .check_write_owned("addComment", &item.uri, comment_uri)?;
        Ok(self
            .tracker()
            .add_comment(comment_uri, None, Some(&soap_text(text)))
            .await?)
    }

    /// Mark a comment of `item` as resolved or unresolved.
    pub async fn set_comment_resolved(
        &self,
        item: &ItemRef,
        comment_uri: &str,
        resolved: bool,
    ) -> Result<()> {
        self.polarion
            .check_write_owned("setResolvedComment", &item.uri, comment_uri)?;
        Ok(self
            .tracker()
            .set_resolved_comment(comment_uri, resolved)
            .await?)
    }

    /// Replace the tags of a comment of `item`.
    pub async fn set_comment_tags(
        &self,
        item: &ItemRef,
        comment_uri: &str,
        tags: &[&str],
    ) -> Result<()> {
        self.polarion
            .check_write_owned("setCommentTags", &item.uri, comment_uri)?;
        let tags: Vec<soap::EnumOptionId> = tags.iter().map(|t| option_id(t)).collect();
        Ok(self.tracker().set_comment_tags(comment_uri, &tags).await?)
    }

    // ---- links

    /// Link to another work item with a role (e.g. `parent`, `relates_to`).
    /// Returns whether a link was added.
    pub async fn add_link(&self, item: &ItemRef, role: &str, target: &ItemRef) -> Result<bool> {
        self.polarion
            .check_write_targets("addLinkedItem", &item.uri, &[&target.uri])?;
        let tracker = self.tracker();
        Ok(tracker
            .add_linked_item(&item.uri, &target.uri, &option_id(role))
            .await?)
    }

    /// Returns whether a link was removed.
    pub async fn remove_link(&self, item: &ItemRef, role: &str, target: &ItemRef) -> Result<bool> {
        self.polarion
            .check_write_targets("removeLinkedItem", &item.uri, &[&target.uri])?;
        let tracker = self.tracker();
        Ok(tracker
            .remove_linked_item(&item.uri, &target.uri, &option_id(role))
            .await?)
    }

    /// Add a hyperlink with a role (e.g. `ref_ext`).
    pub async fn add_hyperlink(&self, item: &ItemRef, url: &str, role: &str) -> Result<bool> {
        let tracker = self.writable("addHyperlink", item)?;
        Ok(tracker
            .add_hyperlink(&item.uri, url, &option_id(role))
            .await?)
    }

    pub async fn remove_hyperlink(&self, item: &ItemRef, url: &str) -> Result<bool> {
        let tracker = self.writable("removeHyperlink", item)?;
        Ok(tracker.remove_hyperlink(&item.uri, url).await?)
    }

    /// Link to a work item on another server (OSLC) by its URL.
    pub async fn add_external_link(&self, item: &ItemRef, url: &str, role: &str) -> Result<bool> {
        let tracker = self.unbounded("addExternallyLinkedItem", item)?;
        Ok(tracker
            .add_externally_linked_item(&item.uri, url, &option_id(role))
            .await?)
    }

    pub async fn remove_external_link(
        &self,
        item: &ItemRef,
        url: &str,
        role: &str,
    ) -> Result<bool> {
        let tracker = self.unbounded("removeExternallyLinkedItem", item)?;
        Ok(tracker
            .remove_externally_linked_item(&item.uri, url, &option_id(role))
            .await?)
    }

    /// Link a source repository revision; `repository` `None` is the default repository.
    pub async fn add_linked_revision(
        &self,
        item: &ItemRef,
        repository: Option<&str>,
        revision: &str,
    ) -> Result<bool> {
        let tracker = self.unbounded("addLinkedRevision", item)?;
        Ok(match repository {
            None => tracker.add_linked_revision(&item.uri, revision).await?,
            Some(repo) => {
                tracker
                    .add_external_linked_revision(&item.uri, repo, revision)
                    .await?
            }
        })
    }

    pub async fn remove_linked_revision(
        &self,
        item: &ItemRef,
        repository: Option<&str>,
        revision: &str,
    ) -> Result<bool> {
        let tracker = self.unbounded("removeLinkedRevision", item)?;
        Ok(match repository {
            None => tracker.remove_linked_revision(&item.uri, revision).await?,
            Some(repo) => {
                tracker
                    .remove_external_linked_revision(&item.uri, repo, revision)
                    .await?
            }
        })
    }

    /// Link to another work item, pinned to a revision of it (`None`: HEAD), optionally
    /// marked suspect.
    pub async fn add_link_at_revision(
        &self,
        item: &ItemRef,
        role: &str,
        target: &ItemRef,
        revision: Option<&str>,
        suspect: bool,
    ) -> Result<bool> {
        self.polarion
            .check_write_targets("addLinkedItemWithRev", &item.uri, &[&target.uri])?;
        let tracker = self.tracker();
        Ok(tracker
            .add_linked_item_with_rev(&item.uri, &target.uri, &option_id(role), revision, suspect)
            .await?)
    }

    /// Link an OSLC resource (by URL) with a role and a label.
    pub async fn add_oslc_link(
        &self,
        item: &ItemRef,
        url: &str,
        role: &str,
        label: &str,
    ) -> Result<bool> {
        let tracker = self.unbounded("addLinkedOslcItem", item)?;
        Ok(tracker
            .add_linked_oslc_item(&item.uri, url, &option_id(role), label)
            .await?)
    }

    /// Add a planning constraint (e.g. `startNoEarlierThan`) at a date.
    pub async fn add_planning_constraint(
        &self,
        item: &ItemRef,
        date: &str,
        constraint: &str,
    ) -> Result<bool> {
        let tracker = self.writable("addPlaningContraint", item)?;
        Ok(tracker
            .add_planing_contraint(&item.uri, date, &option_id(constraint))
            .await?)
    }

    pub async fn remove_planning_constraint(
        &self,
        item: &ItemRef,
        date: &str,
        constraint: &str,
    ) -> Result<bool> {
        let tracker = self.writable("removePlaningConstraint", item)?;
        Ok(tracker
            .remove_planing_constraint(&item.uri, date, &option_id(constraint))
            .await?)
    }

    // ---- people and categories

    pub async fn add_assignee(&self, item: &ItemRef, user: &str) -> Result<bool> {
        let tracker = self.writable("addAssignee", item)?;
        Ok(tracker.add_assignee(&item.uri, user).await?)
    }

    pub async fn remove_assignee(&self, item: &ItemRef, user: &str) -> Result<bool> {
        let tracker = self.writable("removeAssignee", item)?;
        Ok(tracker.remove_assignee(&item.uri, user).await?)
    }

    pub async fn add_category(&self, item: &ItemRef, category: &str) -> Result<bool> {
        let tracker = self.writable("addCategory", item)?;
        Ok(tracker.add_category(&item.uri, category).await?)
    }

    pub async fn remove_category(&self, item: &ItemRef, category: &str) -> Result<bool> {
        let tracker = self.writable("removeCategory", item)?;
        Ok(tracker.remove_category(&item.uri, category).await?)
    }

    /// Request an approval from a user.
    pub async fn add_approvee(&self, item: &ItemRef, user: &str) -> Result<()> {
        let tracker = self.writable("addApprovee", item)?;
        Ok(tracker.add_approvee(&item.uri, user).await?)
    }

    pub async fn remove_approvee(&self, item: &ItemRef, user: &str) -> Result<()> {
        let tracker = self.writable("removeApprovee", item)?;
        Ok(tracker.remove_approvee(&item.uri, user).await?)
    }

    /// Set a user's approval status (e.g. `approved`, `disapproved`, `waiting`).
    pub async fn set_approval(&self, item: &ItemRef, user: &str, status: &str) -> Result<()> {
        let tracker = self.writable("editApproval", item)?;
        Ok(tracker
            .edit_approval(&item.uri, user, &option_id(status))
            .await?)
    }

    // ---- attachments

    pub async fn add_attachment(
        &self,
        item: &ItemRef,
        file_name: &str,
        title: &str,
        data: &[u8],
    ) -> Result<()> {
        let tracker = self.writable("createAttachment", item)?;
        Ok(tracker
            .create_attachment(&item.uri, file_name, title, data)
            .await?)
    }

    /// Change an attachment; `None` leaves that part unchanged.
    pub async fn update_attachment(
        &self,
        item: &ItemRef,
        attachment_id: &str,
        file_name: Option<&str>,
        title: Option<&str>,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let tracker = self.writable("updateAttachment", item)?;
        Ok(tracker
            .update_attachment(&item.uri, attachment_id, file_name, title, data)
            .await?)
    }

    pub async fn delete_attachment(&self, item: &ItemRef, attachment_id: &str) -> Result<()> {
        let tracker = self.writable("deleteAttachment", item)?;
        Ok(tracker.delete_attachment(&item.uri, attachment_id).await?)
    }

    // ---- time tracking

    pub async fn add_work_record(&self, item: &ItemRef, record: &NewWorkRecord) -> Result<()> {
        let tracker = self.writable("createWorkRecord", item)?;
        let user = soap::User {
            id: Some(record.user.clone()),
            ..soap::User::default()
        };
        match (&record.kind, &record.comment) {
            (None, None) => {
                tracker
                    .create_work_record(&item.uri, &user, &record.date, &record.time_spent)
                    .await?
            }
            (Some(kind), comment) => {
                tracker
                    .create_work_record_with_type_and_comment(
                        &item.uri,
                        &user,
                        &record.date,
                        &option_id(kind),
                        &record.time_spent,
                        comment.as_deref().unwrap_or(""),
                    )
                    .await?
            }
            (None, Some(_)) => {
                return Err(Error::Invalid(
                    "a work record with a comment needs a type (`kind`)".into(),
                ));
            }
        }
        Ok(())
    }

    pub async fn delete_work_record(&self, item: &ItemRef, record_uri: &str) -> Result<()> {
        self.polarion
            .check_write_owned("deleteWorkRecord", &item.uri, record_uri)?;
        Ok(self
            .tracker()
            .delete_work_record(&item.uri, record_uri)
            .await?)
    }

    // ---- other

    /// Set the title in a language (`None`: the default language); `None` title clears it.
    pub async fn set_localized_title(
        &self,
        item: &ItemRef,
        language: Option<&str>,
        title: Option<&str>,
    ) -> Result<()> {
        let tracker = self.writable("setLocalizedWorkItemTitle", item)?;
        Ok(tracker
            .set_localized_work_item_title(&item.uri, language, title)
            .await?)
    }

    /// Set the description in a language (`None`: the default language); `None` clears it.
    pub async fn set_localized_description(
        &self,
        item: &ItemRef,
        language: Option<&str>,
        description: Option<&Text>,
    ) -> Result<()> {
        let tracker = self.writable("setLocalizedWorkItemDescription", item)?;
        let description = description.map(soap_text);
        Ok(tracker
            .set_localized_work_item_description(&item.uri, language, description.as_ref())
            .await?)
    }

    /// Mark links to this work item as suspect, as configured for the project.
    pub async fn auto_suspect(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.unbounded("doAutoSuspect", item)?;
        Ok(tracker.do_auto_suspect(&item.uri).await?)
    }

    /// Assign the work item according to the project's auto-assignment rules.
    pub async fn auto_assign(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.unbounded("doAutoassign", item)?;
        Ok(tracker.do_autoassign(&item.uri).await?)
    }
}
