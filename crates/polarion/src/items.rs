//! Work items: reads and (guarded) writes.

use polarion_soap::services::TrackerService;
use polarion_soap::types as soap;

use crate::client::{Polarion, not_found};
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
/// Every method that modifies data first checks the item's project against the
/// write allowlist ([`crate::PolarionBuilder::allow_writes`]) and fails with
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

    fn writable(&self, operation: &'static str, item: &ItemRef) -> Result<TrackerService<'a>> {
        self.polarion
            .check_write(operation, item.project.as_deref())?;
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

    /// Work items matching a Lucene query (optionally in a baseline).
    pub async fn query(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let tracker = self.tracker();
        let lucene = query.lucene();
        let list = match query.baseline_revision() {
            None => {
                tracker
                    .query_work_items_limited(
                        &lucene,
                        query.sort_key(),
                        query.field_keys(),
                        query.results_limit(),
                    )
                    .await?
            }
            Some(revision) => {
                tracker
                    .query_work_items_in_baseline_limited(
                        &lucene,
                        query.sort_key(),
                        revision,
                        query.field_keys(),
                        query.results_limit(),
                    )
                    .await?
            }
        };
        Ok(items(list))
    }

    /// References of the work items matching a query; cheaper than [`WorkItems::query`].
    /// The query's fields are ignored.
    pub async fn query_refs(&self, query: &WorkItemQuery) -> Result<Vec<ItemRef>> {
        let tracker = self.tracker();
        let lucene = query.lucene();
        let uris = match query.baseline_revision() {
            None => {
                tracker
                    .query_work_item_uris_limited(&lucene, query.sort_key(), query.results_limit())
                    .await?
            }
            Some(revision) => {
                tracker
                    .query_work_item_uris_in_baseline_limited(
                        &lucene,
                        query.sort_key(),
                        revision,
                        query.results_limit(),
                    )
                    .await?
            }
        };
        Ok(uris.into_iter().map(ItemRef::from_uri).collect())
    }

    /// Work items selected by an SQL query against Polarion's database
    /// (`SELECT ... FROM WORKITEM ...`).
    pub async fn query_sql(&self, sql: &str, fields: &Fields) -> Result<Vec<WorkItem>> {
        let list = self
            .tracker()
            .query_work_items_by_sql(sql, fields.keys())
            .await?;
        Ok(items(list))
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

    // --------------------------------------------------------------- writes

    /// Create a work item; returns its reference.
    pub async fn create(&self, new: &NewWorkItem) -> Result<ItemRef> {
        self.polarion
            .check_write("createWorkItem", Some(&new.project))?;
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
            .check_write("createWorkItemInModule", Some(&new.project))?;
        self.polarion
            .check_write("createWorkItemInModule", document.project.as_deref())?;
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
        Ok(tracker.set_fields_null(&item.uri, keys).await?)
    }

    pub async fn delete(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.writable("deleteWorkItem", item)?;
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
        let tracker = self.writable("moveWorkItemToDocument", item)?;
        self.polarion
            .check_write("moveWorkItemToDocument", document.project.as_deref())?;
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
        let tracker = self.writable("performWorkflowAction", item)?;
        Ok(tracker
            .perform_workflow_action(&item.uri, action_id)
            .await?)
    }

    /// Move the work item to `status` with the available workflow action leading there.
    /// Returns the action performed; [`Error::Invalid`] if no available action leads to `status`.
    pub async fn transition(&self, item: &ItemRef, status: &str) -> Result<WorkflowAction> {
        self.writable("performWorkflowAction", item)?;
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
        let tracker = self.writable("resetWorkflow", item)?;
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

    /// Reply to a comment of `item`; returns the reply's URI.
    pub async fn reply_to_comment(
        &self,
        item: &ItemRef,
        comment_uri: &str,
        text: &Text,
    ) -> Result<String> {
        let tracker = self.writable("addComment", item)?;
        Ok(tracker
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
        let tracker = self.writable("setResolvedComment", item)?;
        Ok(tracker.set_resolved_comment(comment_uri, resolved).await?)
    }

    /// Replace the tags of a comment of `item`.
    pub async fn set_comment_tags(
        &self,
        item: &ItemRef,
        comment_uri: &str,
        tags: &[&str],
    ) -> Result<()> {
        let tracker = self.writable("setCommentTags", item)?;
        let tags: Vec<soap::EnumOptionId> = tags.iter().map(|t| option_id(t)).collect();
        Ok(tracker.set_comment_tags(comment_uri, &tags).await?)
    }

    // ---- links

    /// Link to another work item with a role (e.g. `parent`, `relates_to`).
    /// Returns whether a link was added.
    pub async fn add_link(&self, item: &ItemRef, role: &str, target: &ItemRef) -> Result<bool> {
        let tracker = self.writable("addLinkedItem", item)?;
        Ok(tracker
            .add_linked_item(&item.uri, &target.uri, &option_id(role))
            .await?)
    }

    /// Returns whether a link was removed.
    pub async fn remove_link(&self, item: &ItemRef, role: &str, target: &ItemRef) -> Result<bool> {
        let tracker = self.writable("removeLinkedItem", item)?;
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
        let tracker = self.writable("addExternallyLinkedItem", item)?;
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
        let tracker = self.writable("removeExternallyLinkedItem", item)?;
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
        let tracker = self.writable("addLinkedRevision", item)?;
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
        let tracker = self.writable("removeLinkedRevision", item)?;
        Ok(match repository {
            None => tracker.remove_linked_revision(&item.uri, revision).await?,
            Some(repo) => {
                tracker
                    .remove_external_linked_revision(&item.uri, repo, revision)
                    .await?
            }
        })
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
        let tracker = self.writable("deleteWorkRecord", item)?;
        Ok(tracker.delete_work_record(&item.uri, record_uri).await?)
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
        let tracker = self.writable("doAutoSuspect", item)?;
        Ok(tracker.do_auto_suspect(&item.uri).await?)
    }

    /// Assign the work item according to the project's auto-assignment rules.
    pub async fn auto_assign(&self, item: &ItemRef) -> Result<()> {
        let tracker = self.writable("doAutoassign", item)?;
        Ok(tracker.do_autoassign(&item.uri).await?)
    }
}
