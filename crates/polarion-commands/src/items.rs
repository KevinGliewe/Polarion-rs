//! `items.*`: work items.

use polarion::{NewWorkItem, NewWorkRecord, Polarion, WorkItemUpdate};
use serde_json::{Value, json};

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{bytes, changed, command, out};

const KEY: Param = Param::req(
    "key",
    Kind::String,
    "Field key, e.g. `status` or a custom field id",
);
const ENUM_ID: Param = Param::req("enum-id", Kind::String, "Enumeration id");
const COMMENT: Param = Param::req("comment", Kind::String, "Comment URI");
const ROLE: Param = Param::req(
    "role",
    Kind::String,
    "Link role id, e.g. `parent`, `relates_to`",
);
const TARGET: Param = Param::req("target", Kind::Item, "The linked work item");
const URL: Param = Param::req("url", Kind::String, "URL");
const USER: Param = Param::req("user", Kind::String, "User id");
const LANGUAGE: Param = Param::opt(
    "language",
    Kind::String,
    "Language id (default language if omitted)",
);
const ATTACHMENT: Param = Param::req("attachment", Kind::String, "Attachment id");

pub(crate) const COMMANDS: &[Command] = &[
    // ---- reads
    command!(
        "items",
        "get",
        false,
        Output::Object,
        "Get a work item with all fields (or only `fields`/`custom`)",
        [ITEM, FIELDS, CUSTOM],
        get
    ),
    command!(
        "items",
        "get-in-revision",
        false,
        Output::Object,
        "Get a work item as it was in a repository revision",
        [
            ITEM,
            Param::req("revision", Kind::String, "Repository revision"),
            FIELDS,
            CUSTOM
        ],
        get_in_revision
    ),
    command!(
        "items",
        "query",
        false,
        Output::Table(ITEM_COLUMNS),
        "Search work items with a Lucene query",
        [
            QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS, CUSTOM, BASELINE, REVISION
        ],
        query
    ),
    command!(
        "items",
        "query-refs",
        false,
        Output::Table(REF_COLUMNS),
        "Search work items, returning only their references (cheaper)",
        [QUERY, PROJECT, SORT, LIMIT, ALL, BASELINE],
        query_refs
    ),
    command!(
        "items",
        "query-sql",
        false,
        Output::Table(ITEM_COLUMNS),
        "Select work items with SQL",
        [SQL, FIELDS, CUSTOM, BASELINE],
        query_sql
    ),
    command!(
        "items",
        "query-refs-sql",
        false,
        Output::Table(REF_COLUMNS),
        "Select work item references with SQL",
        [SQL, BASELINE],
        query_refs_sql
    ),
    command!(
        "items",
        "count",
        false,
        Output::Scalar,
        "Count work items matching a Lucene query",
        [QUERY, PROJECT],
        count
    ),
    command!(
        "items",
        "links",
        false,
        Output::Links,
        "Links of a work item in both directions",
        [ITEM],
        links
    ),
    command!(
        "items",
        "back-links",
        false,
        Output::Object,
        "Links from other work items to this one",
        [ITEM],
        back_links
    ),
    command!(
        "items",
        "history",
        false,
        Output::Table(CHANGE_COLUMNS),
        "The change history of a work item",
        [ITEM],
        history
    ),
    command!(
        "items",
        "revisions",
        false,
        Output::Lines,
        "Repository revisions in which a work item changed",
        [ITEM],
        revisions
    ),
    command!(
        "items",
        "actions",
        false,
        Output::Table(ACTION_COLUMNS),
        "Workflow actions available now",
        [ITEM],
        actions
    ),
    command!(
        "items",
        "unavailable-actions",
        false,
        Output::Table(ACTION_COLUMNS),
        "Workflow actions not available now, with the reason",
        [ITEM],
        unavailable_actions
    ),
    command!(
        "items",
        "initial-action",
        false,
        Output::Object,
        "The workflow action performed on creation",
        [ITEM],
        initial_action
    ),
    command!(
        "items",
        "allowed-assignees",
        false,
        Output::Table(USER_COLUMNS),
        "Users that may be assigned",
        [ITEM],
        allowed_assignees
    ),
    command!(
        "items",
        "allowed-approvers",
        false,
        Output::Table(USER_COLUMNS),
        "Users that may approve",
        [ITEM],
        allowed_approvers
    ),
    command!(
        "items",
        "field-options",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options a field of this work item can take",
        [ITEM, KEY],
        field_options
    ),
    command!(
        "items",
        "filtered-field-options",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of a dependent field, filtered by the controlling option",
        [
            ITEM,
            KEY,
            Param::opt("filter", Kind::String, "Option id of the controlling field")
        ],
        filtered_field_options
    ),
    command!(
        "items",
        "field-option",
        false,
        Output::Object,
        "One option of a field",
        [ITEM, KEY, Param::req("option", Kind::String, "Option id")],
        field_option
    ),
    command!(
        "items",
        "available-option-ids",
        false,
        Output::Lines,
        "Ids of the options of a field available now",
        [ITEM, KEY],
        available_option_ids
    ),
    command!(
        "items",
        "enum-options",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of an enumeration (by enum id) this work item can use",
        [ITEM, ENUM_ID],
        enum_options
    ),
    command!(
        "items",
        "enum-option",
        false,
        Output::Object,
        "One option of an enumeration (by enum id)",
        [
            ITEM,
            ENUM_ID,
            Param::req("option", Kind::String, "Option id")
        ],
        enum_option
    ),
    command!(
        "items",
        "available-enum-option-ids",
        false,
        Output::Lines,
        "Ids of the options of an enumeration available now",
        [ITEM, ENUM_ID],
        available_enum_option_ids
    ),
    command!(
        "items",
        "custom-field",
        false,
        Output::Object,
        "The value of one custom field",
        [ITEM, KEY],
        custom_field
    ),
    command!(
        "items",
        "custom-field-keys",
        false,
        Output::Lines,
        "Keys of the custom fields defined for a work item",
        [ITEM],
        custom_field_keys
    ),
    command!(
        "items",
        "attachment",
        false,
        Output::Bytes,
        "Download an attachment",
        [ITEM, ATTACHMENT],
        attachment
    ),
    command!(
        "items",
        "localized-title",
        false,
        Output::Scalar,
        "The title in a language",
        [ITEM, LANGUAGE],
        localized_title
    ),
    command!(
        "items",
        "localized-description",
        false,
        Output::Object,
        "The description in a language",
        [ITEM, LANGUAGE],
        localized_description
    ),
    command!(
        "items",
        "comment-resolved",
        false,
        Output::Scalar,
        "Whether a comment is in a resolved thread",
        [Param::pos("comment", Kind::String, "Comment URI")],
        comment_resolved
    ),
    command!(
        "items",
        "linked-resources",
        false,
        Output::Object,
        "Source code resources linked to a work item",
        [ITEM],
        linked_resources
    ),
    // ---- writes
    command!(
        "items",
        "create",
        true,
        Output::Object,
        "Create a work item (optionally inside a document, under a parent)",
        [
            PROJECT,
            Param::req(
                "type",
                Kind::String,
                "Work item type id, e.g. `requirement`"
            ),
            Param::req("title", Kind::String, "Title"),
            Param::opt("description", Kind::Text, "Description"),
            Param::opt("priority", Kind::String, "Priority id"),
            Param::opt("severity", Kind::String, "Severity id"),
            Param::opt("due-date", Kind::String, "Due date (YYYY-MM-DD)"),
            Param::opt(
                "initial-estimate",
                Kind::String,
                "Initial estimate, e.g. `2d`"
            ),
            Param::opt("remaining-estimate", Kind::String, "Remaining estimate"),
            Param::opt(
                "custom-fields",
                Kind::Json,
                "Custom field values by key (typed by the field definitions)"
            ),
            Param::opt("document", Kind::Document, "Create inside this document"),
            Param::opt("parent", Kind::Item, "Parent work item inside the document"),
        ],
        create
    ),
    command!(
        "items",
        "update",
        true,
        Output::Done,
        "Change fields of a work item (status changes go through `transition`)",
        [
            ITEM,
            Param::opt("title", Kind::String, "Title"),
            Param::opt("description", Kind::Text, "Description"),
            Param::opt("priority", Kind::String, "Priority id"),
            Param::opt("severity", Kind::String, "Severity id"),
            Param::opt("resolution", Kind::String, "Resolution id"),
            Param::opt("due-date", Kind::String, "Due date (YYYY-MM-DD)"),
            Param::opt("initial-estimate", Kind::String, "Initial estimate"),
            Param::opt("remaining-estimate", Kind::String, "Remaining estimate"),
            Param::opt("time-spent", Kind::String, "Time spent"),
            Param::opt(
                "custom-fields",
                Kind::Json,
                "Custom field values by key; null clears a field"
            ),
            Param::opt("clear", Kind::StringList, "Standard field keys to clear"),
        ],
        update
    ),
    command!(
        "items",
        "set-custom-field",
        true,
        Output::Done,
        "Set (or with null clear) one custom field",
        [
            ITEM,
            KEY,
            Param::req(
                "value",
                Kind::Any,
                "The value (typed by the field definition); null clears"
            )
        ],
        set_custom_field
    ),
    command!(
        "items",
        "clear-fields",
        true,
        Output::Done,
        "Clear fields by key",
        [ITEM, Param::req("fields", Kind::StringList, "Field keys")],
        clear_fields
    ),
    command!(
        "items",
        "delete",
        true,
        Output::Done,
        "Delete a work item",
        [ITEM],
        delete
    ),
    command!(
        "items",
        "move-to-document",
        true,
        Output::Done,
        "Move a work item into (or within) a document",
        [
            ITEM,
            Param::req("document", Kind::Document, "Target document"),
            Param::opt(
                "parent",
                Kind::Item,
                "Parent work item (top level if omitted)"
            ),
            Param::opt(
                "position",
                Kind::Integer,
                "Position among the parent's children (end if omitted)"
            ),
            Param::opt(
                "retain-flow",
                Kind::Boolean,
                "Keep the position in the document flow"
            ),
        ],
        move_to_document
    ),
    command!(
        "items",
        "perform-action",
        true,
        Output::Done,
        "Perform a workflow action by id (see `actions`)",
        [ITEM, Param::req("action", Kind::Integer, "Action id")],
        perform_action
    ),
    command!(
        "items",
        "transition",
        true,
        Output::Object,
        "Move a work item to a status via the matching workflow action",
        [ITEM, Param::req("status", Kind::String, "Target status id")],
        transition
    ),
    command!(
        "items",
        "reset-workflow",
        true,
        Output::Done,
        "Reset the workflow to the initial status",
        [ITEM],
        reset_workflow
    ),
    command!(
        "items",
        "add-comment",
        true,
        Output::Scalar,
        "Comment on a work item; returns the comment URI",
        [ITEM, TEXT, Param::opt("title", Kind::String, "Title")],
        add_comment
    ),
    command!(
        "items",
        "add-restricted-comment",
        true,
        Output::Scalar,
        "Comment visible only to some users or roles",
        [
            ITEM,
            TEXT,
            Param::req("title", Kind::String, "Title"),
            Param::req("visible-to", Kind::StringList, "User ids or roles")
        ],
        add_restricted_comment
    ),
    command!(
        "items",
        "reply-to-comment",
        true,
        Output::Scalar,
        "Reply to a comment",
        [ITEM, COMMENT, TEXT],
        reply_to_comment
    ),
    command!(
        "items",
        "set-comment-resolved",
        true,
        Output::Done,
        "Mark a comment resolved or unresolved",
        [
            ITEM,
            COMMENT,
            Param::req("resolved", Kind::Boolean, "Resolved")
        ],
        set_comment_resolved
    ),
    command!(
        "items",
        "set-comment-tags",
        true,
        Output::Done,
        "Replace the tags of a comment",
        [
            ITEM,
            COMMENT,
            Param::req("tags", Kind::StringList, "Tag ids")
        ],
        set_comment_tags
    ),
    command!(
        "items",
        "add-link",
        true,
        Output::Done,
        "Link to another work item",
        [ITEM, ROLE, TARGET],
        add_link
    ),
    command!(
        "items",
        "remove-link",
        true,
        Output::Done,
        "Remove a link",
        [ITEM, ROLE, TARGET],
        remove_link
    ),
    command!(
        "items",
        "add-link-at-revision",
        true,
        Output::Done,
        "Link to another work item pinned to a revision, optionally suspect",
        [
            ITEM,
            ROLE,
            TARGET,
            Param::opt("revision", Kind::String, "Revision (HEAD if omitted)"),
            Param::opt("suspect", Kind::Boolean, "Mark suspect")
        ],
        add_link_at_revision
    ),
    command!(
        "items",
        "add-hyperlink",
        true,
        Output::Done,
        "Add a hyperlink",
        [ITEM, URL, ROLE],
        add_hyperlink
    ),
    command!(
        "items",
        "remove-hyperlink",
        true,
        Output::Done,
        "Remove a hyperlink",
        [ITEM, URL],
        remove_hyperlink
    ),
    command!(
        "items",
        "add-external-link",
        true,
        Output::Done,
        "Link a work item on another server (OSLC) by URL",
        [ITEM, URL, ROLE],
        add_external_link
    ),
    command!(
        "items",
        "remove-external-link",
        true,
        Output::Done,
        "Remove an external link",
        [ITEM, URL, ROLE],
        remove_external_link
    ),
    command!(
        "items",
        "add-oslc-link",
        true,
        Output::Done,
        "Link an OSLC resource with a label",
        [ITEM, URL, ROLE, Param::req("label", Kind::String, "Label")],
        add_oslc_link
    ),
    command!(
        "items",
        "add-linked-revision",
        true,
        Output::Done,
        "Link a repository revision",
        [
            ITEM,
            Param::req("revision", Kind::String, "Revision"),
            Param::opt(
                "repository",
                Kind::String,
                "Repository (default repository if omitted)"
            )
        ],
        add_linked_revision
    ),
    command!(
        "items",
        "remove-linked-revision",
        true,
        Output::Done,
        "Unlink a repository revision",
        [
            ITEM,
            Param::req("revision", Kind::String, "Revision"),
            Param::opt("repository", Kind::String, "Repository")
        ],
        remove_linked_revision
    ),
    command!(
        "items",
        "add-planning-constraint",
        true,
        Output::Done,
        "Add a planning constraint",
        [
            ITEM,
            Param::req("date", Kind::String, "Date"),
            Param::req("constraint", Kind::String, "Constraint id")
        ],
        add_planning_constraint
    ),
    command!(
        "items",
        "remove-planning-constraint",
        true,
        Output::Done,
        "Remove a planning constraint",
        [
            ITEM,
            Param::req("date", Kind::String, "Date"),
            Param::req("constraint", Kind::String, "Constraint id")
        ],
        remove_planning_constraint
    ),
    command!(
        "items",
        "add-assignee",
        true,
        Output::Done,
        "Assign a user",
        [ITEM, USER],
        add_assignee
    ),
    command!(
        "items",
        "remove-assignee",
        true,
        Output::Done,
        "Unassign a user",
        [ITEM, USER],
        remove_assignee
    ),
    command!(
        "items",
        "add-category",
        true,
        Output::Done,
        "Add a category",
        [ITEM, Param::req("category", Kind::String, "Category id")],
        add_category
    ),
    command!(
        "items",
        "remove-category",
        true,
        Output::Done,
        "Remove a category",
        [ITEM, Param::req("category", Kind::String, "Category id")],
        remove_category
    ),
    command!(
        "items",
        "add-approvee",
        true,
        Output::Done,
        "Request an approval from a user",
        [ITEM, USER],
        add_approvee
    ),
    command!(
        "items",
        "remove-approvee",
        true,
        Output::Done,
        "Remove an approver",
        [ITEM, USER],
        remove_approvee
    ),
    command!(
        "items",
        "set-approval",
        true,
        Output::Done,
        "Set a user's approval status",
        [
            ITEM,
            USER,
            Param::req(
                "status",
                Kind::String,
                "e.g. `approved`, `disapproved`, `waiting`"
            )
        ],
        set_approval
    ),
    command!(
        "items",
        "add-attachment",
        true,
        Output::Done,
        "Attach a file",
        [
            ITEM,
            Param::req("file-name", Kind::String, "File name"),
            Param::req("title", Kind::String, "Title"),
            Param::req("data", Kind::Bytes, "Content")
        ],
        add_attachment
    ),
    command!(
        "items",
        "update-attachment",
        true,
        Output::Done,
        "Change an attachment's file name, title or content",
        [
            ITEM,
            ATTACHMENT,
            Param::opt("file-name", Kind::String, "File name"),
            Param::opt("title", Kind::String, "Title"),
            Param::opt("data", Kind::Bytes, "Content")
        ],
        update_attachment
    ),
    command!(
        "items",
        "delete-attachment",
        true,
        Output::Done,
        "Delete an attachment",
        [ITEM, ATTACHMENT],
        delete_attachment
    ),
    command!(
        "items",
        "add-work-record",
        true,
        Output::Done,
        "Book time",
        [
            ITEM,
            USER,
            Param::req("date", Kind::String, "Date (YYYY-MM-DD)"),
            Param::req("time-spent", Kind::String, "Duration, e.g. `2h`"),
            Param::opt(
                "type",
                Kind::String,
                "Work record type id (needed with a comment)"
            ),
            Param::opt("comment", Kind::String, "Comment"),
        ],
        add_work_record
    ),
    command!(
        "items",
        "delete-work-record",
        true,
        Output::Done,
        "Delete a work record",
        [ITEM, Param::req("record", Kind::String, "Work record URI")],
        delete_work_record
    ),
    command!(
        "items",
        "set-localized-title",
        true,
        Output::Done,
        "Set the title in a language",
        [
            ITEM,
            LANGUAGE,
            Param::opt("title", Kind::String, "Title (clears if omitted)")
        ],
        set_localized_title
    ),
    command!(
        "items",
        "set-localized-description",
        true,
        Output::Done,
        "Set the description in a language",
        [
            ITEM,
            LANGUAGE,
            Param::opt("description", Kind::Text, "Description (clears if omitted)")
        ],
        set_localized_description
    ),
    command!(
        "items",
        "auto-suspect",
        true,
        Output::Done,
        "Mark links to the work item suspect",
        [ITEM],
        auto_suspect
    ),
    command!(
        "items",
        "auto-assign",
        true,
        Output::Done,
        "Assign by the project's rules",
        [ITEM],
        auto_assign
    ),
];

// ---- reads

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let item = a.item("item")?;
    match a.opt_fields()? {
        None => out(p.items().get(&item).await?),
        Some(fields) => out(p.items().get_fields(&item, &fields).await?),
    }
}

async fn get_in_revision(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p
        .items()
        .get_in_revision(&a.item("item")?, &a.str("revision")?, fields.as_ref())
        .await?)
}

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().query(&a.query()?).await?)
}

async fn query_refs(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().query_refs(&a.query()?).await?)
}

async fn query_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let baseline = a.opt_str("baseline")?;
    out(p
        .items()
        .query_sql(&a.str("sql")?, &a.fields()?, baseline.as_deref())
        .await?)
}

async fn query_refs_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let baseline = a.opt_str("baseline")?;
    out(p
        .items()
        .query_refs_sql(&a.str("sql")?, baseline.as_deref())
        .await?)
}

async fn count(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().count(&a.query()?).await?)
}

async fn links(p: &Polarion, a: &Args) -> Result<Value> {
    let item = p.items().get(&a.item("item")?).await?;
    let incoming = match item.reference() {
        Some(r) => p.items().back_links(&r).await?,
        None => Vec::new(),
    };
    out(json!({"outgoing": item.links, "incoming": incoming}))
}

async fn back_links(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().back_links(&a.item("item")?).await?)
}

async fn history(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().history(&a.item("item")?).await?)
}

async fn revisions(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().revisions(&a.item("item")?).await?)
}

async fn actions(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().actions(&a.item("item")?).await?)
}

async fn unavailable_actions(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().unavailable_actions(&a.item("item")?).await?)
}

async fn initial_action(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().initial_action(&a.item("item")?).await?)
}

async fn allowed_assignees(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().allowed_assignees(&a.item("item")?).await?)
}

async fn allowed_approvers(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().allowed_approvers(&a.item("item")?).await?)
}

async fn field_options(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .field_options(&a.item("item")?, &a.str("key")?)
        .await?)
}

async fn filtered_field_options(p: &Polarion, a: &Args) -> Result<Value> {
    let filter = a.opt_str("filter")?;
    out(p
        .items()
        .filtered_field_options(&a.item("item")?, &a.str("key")?, filter.as_deref())
        .await?)
}

async fn field_option(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .field_option(&a.item("item")?, &a.str("key")?, &a.str("option")?)
        .await?)
}

async fn available_option_ids(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .available_option_ids(&a.item("item")?, &a.str("key")?)
        .await?)
}

async fn enum_options(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .enum_options(&a.item("item")?, &a.str("enum-id")?)
        .await?)
}

async fn enum_option(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .enum_option(&a.item("item")?, &a.str("enum-id")?, &a.str("option")?)
        .await?)
}

async fn available_enum_option_ids(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .available_enum_option_ids(&a.item("item")?, &a.str("enum-id")?)
        .await?)
}

async fn custom_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .custom_field(&a.item("item")?, &a.str("key")?)
        .await?)
}

async fn custom_field_keys(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().custom_field_keys(&a.item("item")?).await?)
}

async fn attachment(p: &Polarion, a: &Args) -> Result<Value> {
    bytes(
        &p.items()
            .attachment(&a.item("item")?, &a.str("attachment")?)
            .await?,
    )
}

async fn localized_title(p: &Polarion, a: &Args) -> Result<Value> {
    let language = a.opt_str("language")?;
    out(p
        .items()
        .localized_title(&a.item("item")?, language.as_deref())
        .await?)
}

async fn localized_description(p: &Polarion, a: &Args) -> Result<Value> {
    let language = a.opt_str("language")?;
    out(p
        .items()
        .localized_description(&a.item("item")?, language.as_deref())
        .await?)
}

async fn comment_resolved(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().is_comment_resolved(&a.str("comment")?).await?)
}

async fn linked_resources(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.items().linked_resources(&a.item("item")?).await?)
}

// ---- writes

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let project = a.project()?;
    let kind = a.str("type")?;
    let custom = a.object("custom-fields")?;
    let definitions = if custom.is_empty() {
        Vec::new()
    } else {
        p.schema().custom_fields(&project, &kind).await?
    };
    let mut new = NewWorkItem::new(&project, &kind, a.str("title")?);
    new.description = a.opt_text("description")?;
    new.priority = a.opt_str("priority")?;
    new.severity = a.opt_str("severity")?;
    new.due_date = a.opt_str("due-date")?;
    new.initial_estimate = a.opt_str("initial-estimate")?;
    new.remaining_estimate = a.opt_str("remaining-estimate")?;
    new.custom_fields = a
        .custom_values("custom-fields", &definitions)?
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect();
    let created = match a.get("document") {
        Some(_) => {
            let parent = a.opt_item("parent")?;
            p.items()
                .create_in_document(&new, &a.document("document")?, parent.as_ref())
                .await?
        }
        None => p.items().create(&new).await?,
    };
    out(json!({"item": created, "uri": created.uri}))
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let item = a.item("item")?;
    let definitions = if a.object("custom-fields")?.is_empty() {
        Vec::new()
    } else {
        p.schema().custom_fields_of(&item).await?
    };
    let update = WorkItemUpdate {
        title: a.opt_str("title")?,
        description: a.opt_text("description")?,
        priority: a.opt_str("priority")?,
        severity: a.opt_str("severity")?,
        resolution: a.opt_str("resolution")?,
        due_date: a.opt_str("due-date")?,
        initial_estimate: a.opt_str("initial-estimate")?,
        remaining_estimate: a.opt_str("remaining-estimate")?,
        time_spent: a.opt_str("time-spent")?,
        custom_fields: a.custom_values("custom-fields", &definitions)?,
        clear: a.list("clear")?,
    };
    p.items().update(&item, &update).await?;
    Ok(Value::Null)
}

async fn set_custom_field(p: &Polarion, a: &Args) -> Result<Value> {
    let item = a.item("item")?;
    let key = a.str("key")?;
    let raw = a.get("value").cloned().unwrap_or(Value::Null);
    let value = if raw.is_null() {
        None
    } else {
        let definition = p.schema().custom_field_of(&item, &key).await?;
        polarion::FieldValue::coerce(&raw, &definition)?
    };
    p.items()
        .set_custom_field(&item, &key, value.as_ref())
        .await?;
    Ok(Value::Null)
}

async fn clear_fields(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .clear_fields(&a.item("item")?, &a.list("fields")?)
        .await?;
    Ok(Value::Null)
}

async fn delete(p: &Polarion, a: &Args) -> Result<Value> {
    p.items().delete(&a.item("item")?).await?;
    Ok(Value::Null)
}

async fn move_to_document(p: &Polarion, a: &Args) -> Result<Value> {
    let parent = a.opt_item("parent")?;
    p.items()
        .move_to_document(
            &a.item("item")?,
            &a.document("document")?,
            parent.as_ref(),
            a.opt_u32("position")?,
            a.bool("retain-flow")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn perform_action(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .perform_action(&a.item("item")?, a.i32("action")?)
        .await?;
    Ok(Value::Null)
}

async fn transition(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .transition(&a.item("item")?, &a.str("status")?)
        .await?)
}

async fn reset_workflow(p: &Polarion, a: &Args) -> Result<Value> {
    p.items().reset_workflow(&a.item("item")?).await?;
    Ok(Value::Null)
}

async fn add_comment(p: &Polarion, a: &Args) -> Result<Value> {
    let title = a.opt_str("title")?;
    out(p
        .items()
        .add_comment(&a.item("item")?, title.as_deref(), &a.text("text")?)
        .await?)
}

async fn add_restricted_comment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .add_restricted_comment(
            &a.item("item")?,
            &a.str("title")?,
            &a.text("text")?,
            &a.list("visible-to")?,
        )
        .await?)
}

async fn reply_to_comment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .items()
        .reply_to_comment(&a.item("item")?, &a.str("comment")?, &a.text("text")?)
        .await?)
}

async fn set_comment_resolved(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .set_comment_resolved(&a.item("item")?, &a.str("comment")?, a.bool("resolved")?)
        .await?;
    Ok(Value::Null)
}

async fn set_comment_tags(p: &Polarion, a: &Args) -> Result<Value> {
    let tags = a.list("tags")?;
    let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
    p.items()
        .set_comment_tags(&a.item("item")?, &a.str("comment")?, &tags)
        .await?;
    Ok(Value::Null)
}

async fn add_link(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_link(&a.item("item")?, &a.str("role")?, &a.item("target")?)
            .await?,
    )
}

async fn remove_link(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_link(&a.item("item")?, &a.str("role")?, &a.item("target")?)
            .await?,
    )
}

async fn add_link_at_revision(p: &Polarion, a: &Args) -> Result<Value> {
    let revision = a.opt_str("revision")?;
    changed(
        p.items()
            .add_link_at_revision(
                &a.item("item")?,
                &a.str("role")?,
                &a.item("target")?,
                revision.as_deref(),
                a.bool("suspect")?,
            )
            .await?,
    )
}

async fn add_hyperlink(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_hyperlink(&a.item("item")?, &a.str("url")?, &a.str("role")?)
            .await?,
    )
}

async fn remove_hyperlink(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_hyperlink(&a.item("item")?, &a.str("url")?)
            .await?,
    )
}

async fn add_external_link(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_external_link(&a.item("item")?, &a.str("url")?, &a.str("role")?)
            .await?,
    )
}

async fn remove_external_link(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_external_link(&a.item("item")?, &a.str("url")?, &a.str("role")?)
            .await?,
    )
}

async fn add_oslc_link(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_oslc_link(
                &a.item("item")?,
                &a.str("url")?,
                &a.str("role")?,
                &a.str("label")?,
            )
            .await?,
    )
}

async fn add_linked_revision(p: &Polarion, a: &Args) -> Result<Value> {
    let repository = a.opt_str("repository")?;
    changed(
        p.items()
            .add_linked_revision(&a.item("item")?, repository.as_deref(), &a.str("revision")?)
            .await?,
    )
}

async fn remove_linked_revision(p: &Polarion, a: &Args) -> Result<Value> {
    let repository = a.opt_str("repository")?;
    changed(
        p.items()
            .remove_linked_revision(&a.item("item")?, repository.as_deref(), &a.str("revision")?)
            .await?,
    )
}

async fn add_planning_constraint(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_planning_constraint(&a.item("item")?, &a.str("date")?, &a.str("constraint")?)
            .await?,
    )
}

async fn remove_planning_constraint(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_planning_constraint(&a.item("item")?, &a.str("date")?, &a.str("constraint")?)
            .await?,
    )
}

async fn add_assignee(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_assignee(&a.item("item")?, &a.str("user")?)
            .await?,
    )
}

async fn remove_assignee(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_assignee(&a.item("item")?, &a.str("user")?)
            .await?,
    )
}

async fn add_category(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .add_category(&a.item("item")?, &a.str("category")?)
            .await?,
    )
}

async fn remove_category(p: &Polarion, a: &Args) -> Result<Value> {
    changed(
        p.items()
            .remove_category(&a.item("item")?, &a.str("category")?)
            .await?,
    )
}

async fn add_approvee(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .add_approvee(&a.item("item")?, &a.str("user")?)
        .await?;
    Ok(Value::Null)
}

async fn remove_approvee(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .remove_approvee(&a.item("item")?, &a.str("user")?)
        .await?;
    Ok(Value::Null)
}

async fn set_approval(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .set_approval(&a.item("item")?, &a.str("user")?, &a.str("status")?)
        .await?;
    Ok(Value::Null)
}

async fn add_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .add_attachment(
            &a.item("item")?,
            &a.str("file-name")?,
            &a.str("title")?,
            &a.bytes("data")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn update_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    let (file_name, title, data) = (
        a.opt_str("file-name")?,
        a.opt_str("title")?,
        a.opt_bytes("data")?,
    );
    p.items()
        .update_attachment(
            &a.item("item")?,
            &a.str("attachment")?,
            file_name.as_deref(),
            title.as_deref(),
            data.as_deref(),
        )
        .await?;
    Ok(Value::Null)
}

async fn delete_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .delete_attachment(&a.item("item")?, &a.str("attachment")?)
        .await?;
    Ok(Value::Null)
}

async fn add_work_record(p: &Polarion, a: &Args) -> Result<Value> {
    let record = NewWorkRecord {
        user: a.str("user")?,
        date: a.str("date")?,
        time_spent: a.str("time-spent")?,
        kind: a.opt_str("type")?,
        comment: a.opt_str("comment")?,
    };
    p.items().add_work_record(&a.item("item")?, &record).await?;
    Ok(Value::Null)
}

async fn delete_work_record(p: &Polarion, a: &Args) -> Result<Value> {
    p.items()
        .delete_work_record(&a.item("item")?, &a.str("record")?)
        .await?;
    Ok(Value::Null)
}

async fn set_localized_title(p: &Polarion, a: &Args) -> Result<Value> {
    let (language, title) = (a.opt_str("language")?, a.opt_str("title")?);
    p.items()
        .set_localized_title(&a.item("item")?, language.as_deref(), title.as_deref())
        .await?;
    Ok(Value::Null)
}

async fn set_localized_description(p: &Polarion, a: &Args) -> Result<Value> {
    let (language, description) = (a.opt_str("language")?, a.opt_text("description")?);
    p.items()
        .set_localized_description(&a.item("item")?, language.as_deref(), description.as_ref())
        .await?;
    Ok(Value::Null)
}

async fn auto_suspect(p: &Polarion, a: &Args) -> Result<Value> {
    p.items().auto_suspect(&a.item("item")?).await?;
    Ok(Value::Null)
}

async fn auto_assign(p: &Polarion, a: &Args) -> Result<Value> {
    p.items().auto_assign(&a.item("item")?).await?;
    Ok(Value::Null)
}
