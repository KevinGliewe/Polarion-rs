//! `items.*`: work items.

use polarion::polarion_markdown::{RawHtml, WorkItemIds};
use polarion::{
    MarkdownReadOptions, MarkdownWriteOptions, NewWorkItem, NewWorkRecord, WorkItemUpdate,
};
use polarion_commands_derive::cmd;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    ACTION_COLUMNS, CHANGE_COLUMNS, Changed, CommandError, Context, Data, DocumentArg, Done,
    Download, FieldSelection, ITEM_COLUMNS, ItemArg, JsonObject, JsonValue, OPTION_COLUMNS,
    QueryArgs, REF_COLUMNS, Result, RichText, Run, TextArg, USER_COLUMNS,
};

/// Get a work item with all fields (or only `fields`/`custom`)
#[cmd(view = object)]
pub struct Get {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
}

impl Run for Get {
    type Output = polarion::WorkItem;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let item = self.item.resolve(cx)?;
        Ok(
            match FieldSelection::fields(self.fields.as_ref(), &self.custom) {
                None => cx.polarion.items().get(&item).await?,
                Some(fields) => cx.polarion.items().get_fields(&item, &fields).await?,
            },
        )
    }
}

/// Get a work item as it was in a repository revision.
#[cmd(view = object)]
pub struct GetInRevision {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Repository revision
    pub revision: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
}

impl Run for GetInRevision {
    type Output = polarion::WorkItem;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &self.custom);
        Ok(cx
            .polarion
            .items()
            .get_in_revision(&self.item.resolve(cx)?, &self.revision, fields.as_ref())
            .await?)
    }
}

/// Search work items with a Lucene query.
#[cmd(view = table(ITEM_COLUMNS))]
pub struct Query {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Query {
    type Output = Vec<polarion::WorkItem>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.items().query(&self.search.query(cx)).await?)
    }
}

/// Search work items, returning only their references (cheaper)
#[cmd(view = table(REF_COLUMNS))]
pub struct QueryRefs {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for QueryRefs {
    type Output = Vec<polarion::ItemRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .query_refs(&self.search.query(cx))
            .await?)
    }
}

/// Select work items with SQL.
#[cmd(view = table(ITEM_COLUMNS))]
pub struct QuerySql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QuerySql {
    type Output = Vec<polarion::WorkItem>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let baseline = self.baseline;
        Ok(cx
            .polarion
            .items()
            .query_sql(
                &self.sql,
                &FieldSelection::fields(self.fields.as_ref(), &self.custom).unwrap_or_default(),
                baseline.as_deref(),
            )
            .await?)
    }
}

/// Select work item references with SQL.
#[cmd(view = table(REF_COLUMNS))]
pub struct QueryRefsSql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QueryRefsSql {
    type Output = Vec<polarion::ItemRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let baseline = self.baseline;
        Ok(cx
            .polarion
            .items()
            .query_refs_sql(&self.sql, baseline.as_deref())
            .await?)
    }
}

/// Count work items matching a Lucene query.
#[cmd(view = scalar)]
pub struct Count {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Count {
    type Output = u32;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.items().count(&self.search.query(cx)).await?)
    }
}

/// Links of a work item in both directions.
#[cmd(view = links)]
pub struct Links {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

/// A work item's links: `outgoing` from it, `incoming` from other items (each link's `item` is
/// the other end).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ItemLinks {
    pub outgoing: Vec<polarion::Link>,
    pub incoming: Vec<polarion::Link>,
}

impl Run for Links {
    type Output = ItemLinks;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let item = cx.polarion.items().get(&self.item.resolve(cx)?).await?;
        let incoming = match item.reference() {
            Some(r) => cx.polarion.items().back_links(&r).await?,
            None => Vec::new(),
        };
        Ok(ItemLinks {
            outgoing: item.links,
            incoming,
        })
    }
}

/// Links from other work items to this one.
#[cmd(view = object)]
pub struct BackLinks {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for BackLinks {
    type Output = Vec<polarion::Link>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .back_links(&self.item.resolve(cx)?)
            .await?)
    }
}

/// The change history of a work item.
#[cmd(view = table(CHANGE_COLUMNS))]
pub struct History {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for History {
    type Output = Vec<polarion::Change>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.items().history(&self.item.resolve(cx)?).await?)
    }
}

/// Repository revisions in which a work item changed.
#[cmd(view = lines)]
pub struct Revisions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for Revisions {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .revisions(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Workflow actions available now.
#[cmd(view = table(ACTION_COLUMNS))]
pub struct Actions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for Actions {
    type Output = Vec<polarion::WorkflowAction>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.items().actions(&self.item.resolve(cx)?).await?)
    }
}

/// Workflow actions not available now, with the reason.
#[cmd(view = table(ACTION_COLUMNS))]
pub struct UnavailableActions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for UnavailableActions {
    type Output = Vec<polarion::WorkflowAction>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .unavailable_actions(&self.item.resolve(cx)?)
            .await?)
    }
}

/// The workflow action performed on creation.
#[cmd(view = object)]
pub struct InitialAction {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for InitialAction {
    type Output = Option<polarion::WorkflowAction>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .initial_action(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Users that may be assigned.
#[cmd(view = table(USER_COLUMNS))]
pub struct AllowedAssignees {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for AllowedAssignees {
    type Output = Vec<polarion::User>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .allowed_assignees(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Users that may approve.
#[cmd(view = table(USER_COLUMNS))]
pub struct AllowedApprovers {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for AllowedApprovers {
    type Output = Vec<polarion::User>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .allowed_approvers(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Options a field of this work item can take.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct FieldOptions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for FieldOptions {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .field_options(&self.item.resolve(cx)?, &self.key)
            .await?)
    }
}

/// Options of a dependent field, filtered by the controlling option.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct FilteredFieldOptions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
    /// Option id of the controlling field
    pub filter: Option<String>,
}

impl Run for FilteredFieldOptions {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let filter = self.filter;
        Ok(cx
            .polarion
            .items()
            .filtered_field_options(&self.item.resolve(cx)?, &self.key, filter.as_deref())
            .await?)
    }
}

/// One option of a field.
#[cmd(view = object)]
pub struct FieldOption {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
    /// Option id
    pub option: String,
}

impl Run for FieldOption {
    type Output = polarion::EnumOption;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .field_option(&self.item.resolve(cx)?, &self.key, &self.option)
            .await?)
    }
}

/// Ids of the options of a field available now.
#[cmd(view = lines)]
pub struct AvailableOptionIds {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for AvailableOptionIds {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .available_option_ids(&self.item.resolve(cx)?, &self.key)
            .await?)
    }
}

/// Options of an enumeration (by enum id) this work item can use.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct EnumOptions {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for EnumOptions {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .enum_options(&self.item.resolve(cx)?, &self.enum_id)
            .await?)
    }
}

/// One option of an enumeration (by enum id)
#[cmd(view = object)]
pub struct EnumOption {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Enumeration id
    pub enum_id: String,
    /// Option id
    pub option: String,
}

impl Run for EnumOption {
    type Output = polarion::EnumOption;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .enum_option(&self.item.resolve(cx)?, &self.enum_id, &self.option)
            .await?)
    }
}

/// Ids of the options of an enumeration available now.
#[cmd(view = lines)]
pub struct AvailableEnumOptionIds {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for AvailableEnumOptionIds {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .available_enum_option_ids(&self.item.resolve(cx)?, &self.enum_id)
            .await?)
    }
}

/// The value of one custom field.
#[cmd(view = object)]
pub struct CustomField {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for CustomField {
    type Output = Option<polarion::FieldValue>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .custom_field(&self.item.resolve(cx)?, &self.key)
            .await?)
    }
}

/// Keys of the custom fields defined for a work item.
#[cmd(view = lines)]
pub struct CustomFieldKeys {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for CustomFieldKeys {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .custom_field_keys(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Download an attachment.
#[cmd(view = bytes)]
pub struct Attachment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Attachment id
    pub attachment: String,
}

impl Run for Attachment {
    type Output = Download;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Download(
            cx.polarion
                .items()
                .attachment(&self.item.resolve(cx)?, &self.attachment)
                .await?,
        ))
    }
}

/// The title in a language.
#[cmd(view = scalar)]
pub struct LocalizedTitle {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Language id (default language if omitted)
    pub language: Option<String>,
}

impl Run for LocalizedTitle {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let language = self.language;
        Ok(cx
            .polarion
            .items()
            .localized_title(&self.item.resolve(cx)?, language.as_deref())
            .await?)
    }
}

/// The description in a language.
#[cmd(view = object)]
pub struct LocalizedDescription {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Language id (default language if omitted)
    pub language: Option<String>,
}

impl Run for LocalizedDescription {
    type Output = polarion::Text;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let language = self.language;
        Ok(cx
            .polarion
            .items()
            .localized_description(&self.item.resolve(cx)?, language.as_deref())
            .await?)
    }
}

/// A work item's description as Markdown, with its mermaid diagrams as code blocks.
///
/// Mermaid diagrams written by `set-description-markdown` come back as mermaid code blocks.
/// Formatting Markdown cannot express (colors, fonts, alignment, merged cells, ...) stays inline
/// HTML, so writing the Markdown back keeps it, unless `drop-html` is set.
#[cmd(view = text)]
pub struct DescriptionMarkdown {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Drop formatting Markdown cannot express (colors, fonts, merged cells, ...) instead of keeping it as inline HTML
    pub drop_html: bool,
    /// Id prefixes whose work item links are read as plain ids, e.g. `MP` (default: the work item's own; `*`: any; `none`: no link)
    pub id_prefixes: Vec<String>,
}

impl DescriptionMarkdown {
    fn options(&self) -> Result<MarkdownReadOptions> {
        Ok(MarkdownReadOptions {
            raw_html: if self.drop_html {
                RawHtml::Drop
            } else {
                RawHtml::Keep
            },
            work_items: work_item_ids(&self.id_prefixes)?,
        })
    }
}

impl Run for DescriptionMarkdown {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let options = self.options()?;
        Ok(cx
            .polarion
            .items()
            .description_markdown(&self.item.resolve(cx)?, &options)
            .await?)
    }
}

/// `id-prefixes` as the library's option: none, the work item's own prefix (`None`); `*`, any
/// id; `none`, no id. Anything else must be a prefix the conversion recognizes (`MP`, `MY_PROJ`):
/// an uppercase letter, then uppercase letters, digits or `_`.
fn work_item_ids(prefixes: &[String]) -> Result<Option<WorkItemIds>> {
    let prefixes: Vec<&str> = prefixes
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if prefixes.is_empty() {
        return Ok(None);
    }
    if prefixes.contains(&"*") {
        return Ok(Some(WorkItemIds::Any));
    }
    if prefixes.contains(&"none") {
        if prefixes.len() > 1 {
            return Err(CommandError::invalid(
                "id-prefixes: `none` cannot be combined with prefixes",
            ));
        }
        return Ok(Some(WorkItemIds::Off));
    }
    for prefix in &prefixes {
        let mut bytes = prefix.bytes();
        let valid = bytes.next().is_some_and(|b| b.is_ascii_uppercase())
            && prefix.len() > 1
            && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(CommandError::invalid(format!(
                "id-prefixes: `{prefix}` is not a work item id prefix (an uppercase letter, then \
                 uppercase letters, digits or `_`, e.g. `MP` for MP-42; `*`: any id; `none`: no \
                 link)"
            )));
        }
    }
    Ok(Some(WorkItemIds::Prefixes(
        prefixes.into_iter().map(str::to_owned).collect(),
    )))
}

/// The options of the Markdown writes; fenced code is highlighted.
fn write_options(
    id_prefixes: &[String],
    keep_orphans: bool,
    allow_unrendered: bool,
) -> Result<MarkdownWriteOptions> {
    Ok(MarkdownWriteOptions {
        work_items: work_item_ids(id_prefixes)?,
        keep_orphans,
        allow_unrendered,
        ..MarkdownWriteOptions::default()
    })
}

/// Whether a comment is in a resolved thread.
#[cmd(view = scalar)]
pub struct CommentResolved {
    /// Comment URI
    #[positional]
    pub comment: String,
}

impl Run for CommentResolved {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .is_comment_resolved(&self.comment)
            .await?)
    }
}

/// Source code resources linked to a work item.
#[cmd(view = object)]
pub struct LinkedResources {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for LinkedResources {
    type Output = Vec<polarion::soap::types::LinkedResource>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .linked_resources(&self.item.resolve(cx)?)
            .await?)
    }
}

/// Create a work item (optionally inside a document, under a parent)
#[cmd(write, view = object)]
pub struct Create {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type id, e.g. `requirement`
    pub r#type: String,
    /// Title
    pub title: String,
    /// Description
    pub description: Option<RichText>,
    /// Priority id
    pub priority: Option<String>,
    /// Severity id
    pub severity: Option<String>,
    /// Due date (YYYY-MM-DD)
    pub due_date: Option<String>,
    /// Initial estimate, e.g. `2d`
    pub initial_estimate: Option<String>,
    /// Remaining estimate
    pub remaining_estimate: Option<String>,
    /// Custom field values by key (typed by the field definitions)
    pub custom_fields: Option<JsonObject>,
    /// Create inside this document
    pub document: Option<DocumentArg>,
    /// Parent work item inside the document
    pub parent: Option<ItemArg>,
}

/// A new work item: its reference and URI.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Created {
    pub item: polarion::ItemRef,
    pub uri: String,
}

impl Run for Create {
    type Output = Created;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let project = cx.project(self.project.as_deref())?;
        let kind = self.r#type;
        let custom = self.custom_fields.unwrap_or_default().0;
        let definitions = if custom.is_empty() {
            Vec::new()
        } else {
            cx.polarion.schema().custom_fields(&project, &kind).await?
        };
        let mut new = NewWorkItem::new(&project, &kind, self.title);
        new.description = self.description.map(polarion::Text::from);
        new.priority = self.priority;
        new.severity = self.severity;
        new.due_date = self.due_date;
        new.initial_estimate = self.initial_estimate;
        new.remaining_estimate = self.remaining_estimate;
        new.custom_fields = crate::values::custom_values(&custom, &definitions)?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect();
        let created = match &self.document {
            Some(document) => {
                let parent = self.parent.as_ref().map(|i| i.resolve(cx)).transpose()?;
                cx.polarion
                    .items()
                    .create_in_document(&new, &document.resolve(cx)?, parent.as_ref())
                    .await?
            }
            None => cx.polarion.items().create(&new).await?,
        };
        Ok(Created {
            uri: created.uri.clone(),
            item: created,
        })
    }
}

/// Change fields of a work item (status changes go through `transition`)
#[cmd(write, view = done)]
pub struct Update {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Title
    pub title: Option<String>,
    /// Description
    pub description: Option<RichText>,
    /// Priority id
    pub priority: Option<String>,
    /// Severity id
    pub severity: Option<String>,
    /// Resolution id
    pub resolution: Option<String>,
    /// Due date (YYYY-MM-DD)
    pub due_date: Option<String>,
    /// Initial estimate
    pub initial_estimate: Option<String>,
    /// Remaining estimate
    pub remaining_estimate: Option<String>,
    /// Time spent
    pub time_spent: Option<String>,
    /// Custom field values by key; null clears a field
    pub custom_fields: Option<JsonObject>,
    /// Standard field keys to clear
    pub clear: Vec<String>,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let item = self.item.resolve(cx)?;
        let definitions = if self.custom_fields.clone().unwrap_or_default().0.is_empty() {
            Vec::new()
        } else {
            cx.polarion.schema().custom_fields_of(&item).await?
        };
        let update = WorkItemUpdate {
            title: self.title,
            description: self.description.map(polarion::Text::from),
            priority: self.priority,
            severity: self.severity,
            resolution: self.resolution,
            due_date: self.due_date,
            initial_estimate: self.initial_estimate,
            remaining_estimate: self.remaining_estimate,
            time_spent: self.time_spent,
            custom_fields: crate::values::custom_values(
                &self.custom_fields.clone().unwrap_or_default().0,
                &definitions,
            )?,
            clear: self.clear,
        };
        cx.polarion.items().update(&item, &update).await?;
        Ok(Done)
    }
}

/// Replace the description with Markdown (mermaid diagrams become SVG attachments).
///
/// Each mermaid code block is rendered to SVG, attached as `mermaid-<hash>.svg` and shown as an
/// image; a diagram that already has its attachment is not uploaded again. A diagram that cannot
/// be rendered fails the command before anything is written, unless `allow-unrendered` is set.
/// Afterwards the diagram attachments only the old description showed are deleted, unless
/// `keep-orphans` is set. Ids with the work item's own prefix, such as `MP-1`, become links.
#[cmd(write, view = object)]
pub struct SetDescriptionMarkdown {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// The new description: the Markdown text itself
    pub markdown: TextArg,
    /// Keep the diagram attachments the old description showed and the new one does not
    pub keep_orphans: bool,
    /// Write a mermaid diagram that cannot be rendered as a code block, with a warning, instead of failing
    pub allow_unrendered: bool,
    /// Id prefixes whose ids become work item links, e.g. `MP` (default: the work item's own; `*`: any; `none`: no link)
    pub id_prefixes: Vec<String>,
}

impl Run for SetDescriptionMarkdown {
    type Output = polarion::MarkdownUpdate;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let options = write_options(&self.id_prefixes, self.keep_orphans, self.allow_unrendered)?;
        Ok(cx
            .polarion
            .items()
            .set_description_markdown(&self.item.resolve(cx)?, &self.markdown.0, &options)
            .await?)
    }
}

/// Append Markdown to the description as a new paragraph.
///
/// As `set-description-markdown`, except that no attachment is deleted. A `text/plain`
/// description is converted to HTML first. The description is read, extended and written back:
/// a change someone makes in between is lost.
#[cmd(write, view = object)]
pub struct AppendDescriptionMarkdown {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// The Markdown to append: the text itself
    pub markdown: TextArg,
    /// Write a mermaid diagram that cannot be rendered as a code block, with a warning, instead of failing
    pub allow_unrendered: bool,
    /// Id prefixes whose ids become work item links, e.g. `MP` (default: the work item's own; `*`: any; `none`: no link)
    pub id_prefixes: Vec<String>,
}

impl Run for AppendDescriptionMarkdown {
    type Output = polarion::MarkdownUpdate;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let options = write_options(&self.id_prefixes, false, self.allow_unrendered)?;
        Ok(cx
            .polarion
            .items()
            .append_description_markdown(&self.item.resolve(cx)?, &self.markdown.0, &options)
            .await?)
    }
}

/// Set (or with null clear) one custom field.
#[cmd(write, view = done)]
pub struct SetCustomField {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
    /// The value (typed by the field definition); null or absent clears
    pub value: Option<JsonValue>,
}

impl Run for SetCustomField {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let item = self.item.resolve(cx)?;
        let key = self.key;
        let value = match self.value {
            None => None,
            Some(JsonValue(raw)) => {
                let definition = cx.polarion.schema().custom_field_of(&item, &key).await?;
                polarion::FieldValue::coerce(&raw, &definition)?
            }
        };
        cx.polarion
            .items()
            .set_custom_field(&item, &key, value.as_ref())
            .await?;
        Ok(Done)
    }
}

/// Clear fields by key.
#[cmd(write, view = done)]
pub struct ClearFields {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field keys
    pub fields: Vec<String>,
}

impl Run for ClearFields {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .clear_fields(&self.item.resolve(cx)?, &self.fields)
            .await?;
        Ok(Done)
    }
}

/// Delete a work item.
#[cmd(write, view = done)]
pub struct Delete {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for Delete {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion.items().delete(&self.item.resolve(cx)?).await?;
        Ok(Done)
    }
}

/// Move a work item into (or within) a document.
#[cmd(write, view = done)]
pub struct MoveToDocument {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Target document
    pub document: DocumentArg,
    /// Parent work item (top level if omitted)
    pub parent: Option<ItemArg>,
    /// Position among the parent's children (end if omitted)
    pub position: Option<u32>,
    /// Keep the position in the document flow
    pub retain_flow: bool,
}

impl Run for MoveToDocument {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let parent = self.parent.as_ref().map(|i| i.resolve(cx)).transpose()?;
        cx.polarion
            .items()
            .move_to_document(
                &self.item.resolve(cx)?,
                &self.document.resolve(cx)?,
                parent.as_ref(),
                self.position,
                self.retain_flow,
            )
            .await?;
        Ok(Done)
    }
}

/// Perform a workflow action by id (see `actions`)
#[cmd(write, view = done)]
pub struct PerformAction {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Action id
    pub action: i32,
}

impl Run for PerformAction {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .perform_action(&self.item.resolve(cx)?, self.action)
            .await?;
        Ok(Done)
    }
}

/// Move a work item to a status via the matching workflow action.
#[cmd(write, view = object)]
pub struct Transition {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Target status id
    pub status: String,
}

impl Run for Transition {
    type Output = polarion::WorkflowAction;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .transition(&self.item.resolve(cx)?, &self.status)
            .await?)
    }
}

/// Reset the workflow to the initial status.
#[cmd(write, view = done)]
pub struct ResetWorkflow {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for ResetWorkflow {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .reset_workflow(&self.item.resolve(cx)?)
            .await?;
        Ok(Done)
    }
}

/// Comment on a work item; returns the comment URI.
#[cmd(write, view = scalar)]
pub struct AddComment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
    /// Title
    pub title: Option<String>,
}

impl Run for AddComment {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let title = self.title;
        Ok(cx
            .polarion
            .items()
            .add_comment(
                &self.item.resolve(cx)?,
                title.as_deref(),
                &polarion::Text::from(self.text),
            )
            .await?)
    }
}

/// Comment visible only to some users or roles.
#[cmd(write, view = scalar)]
pub struct AddRestrictedComment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
    /// Title
    pub title: String,
    /// User ids or roles
    pub visible_to: Vec<String>,
}

impl Run for AddRestrictedComment {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .add_restricted_comment(
                &self.item.resolve(cx)?,
                &self.title,
                &polarion::Text::from(self.text),
                &self.visible_to,
            )
            .await?)
    }
}

/// Reply to a comment.
#[cmd(write, view = scalar)]
pub struct ReplyToComment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Comment URI
    pub comment: String,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
}

impl Run for ReplyToComment {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .items()
            .reply_to_comment(
                &self.item.resolve(cx)?,
                &self.comment,
                &polarion::Text::from(self.text),
            )
            .await?)
    }
}

/// Mark a comment resolved or unresolved.
#[cmd(write, view = done)]
pub struct SetCommentResolved {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Comment URI
    pub comment: String,
    /// Resolved
    pub resolved: bool,
}

impl Run for SetCommentResolved {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .set_comment_resolved(&self.item.resolve(cx)?, &self.comment, self.resolved)
            .await?;
        Ok(Done)
    }
}

/// Replace the tags of a comment.
#[cmd(write, view = done)]
pub struct SetCommentTags {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Comment URI
    pub comment: String,
    /// Tag ids
    pub tags: Vec<String>,
}

impl Run for SetCommentTags {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let tags = self.tags;
        let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
        cx.polarion
            .items()
            .set_comment_tags(&self.item.resolve(cx)?, &self.comment, &tags)
            .await?;
        Ok(Done)
    }
}

/// Link to another work item.
#[cmd(write, view = done)]
pub struct AddLink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
    /// The linked work item
    pub target: ItemArg,
}

impl Run for AddLink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_link(
                    &self.item.resolve(cx)?,
                    &self.role,
                    &self.target.resolve(cx)?,
                )
                .await?,
        ))
    }
}

/// Remove a link.
#[cmd(write, view = done)]
pub struct RemoveLink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
    /// The linked work item
    pub target: ItemArg,
}

impl Run for RemoveLink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_link(
                    &self.item.resolve(cx)?,
                    &self.role,
                    &self.target.resolve(cx)?,
                )
                .await?,
        ))
    }
}

/// Link to another work item pinned to a revision, optionally suspect.
#[cmd(write, view = done)]
pub struct AddLinkAtRevision {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
    /// The linked work item
    pub target: ItemArg,
    /// Revision (HEAD if omitted)
    pub revision: Option<String>,
    /// Mark suspect
    pub suspect: bool,
}

impl Run for AddLinkAtRevision {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let revision = self.revision;
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_link_at_revision(
                    &self.item.resolve(cx)?,
                    &self.role,
                    &self.target.resolve(cx)?,
                    revision.as_deref(),
                    self.suspect,
                )
                .await?,
        ))
    }
}

/// Add a hyperlink.
#[cmd(write, view = done)]
pub struct AddHyperlink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// URL
    pub url: String,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
}

impl Run for AddHyperlink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_hyperlink(&self.item.resolve(cx)?, &self.url, &self.role)
                .await?,
        ))
    }
}

/// Remove a hyperlink.
#[cmd(write, view = done)]
pub struct RemoveHyperlink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// URL
    pub url: String,
}

impl Run for RemoveHyperlink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_hyperlink(&self.item.resolve(cx)?, &self.url)
                .await?,
        ))
    }
}

/// Link a work item on another server (OSLC) by URL.
#[cmd(write, view = done)]
pub struct AddExternalLink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// URL
    pub url: String,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
}

impl Run for AddExternalLink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_external_link(&self.item.resolve(cx)?, &self.url, &self.role)
                .await?,
        ))
    }
}

/// Remove an external link.
#[cmd(write, view = done)]
pub struct RemoveExternalLink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// URL
    pub url: String,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
}

impl Run for RemoveExternalLink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_external_link(&self.item.resolve(cx)?, &self.url, &self.role)
                .await?,
        ))
    }
}

/// Link an OSLC resource with a label.
#[cmd(write, view = done)]
pub struct AddOslcLink {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// URL
    pub url: String,
    /// Link role id, e.g. `parent`, `relates_to`
    pub role: String,
    /// Label
    pub label: String,
}

impl Run for AddOslcLink {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_oslc_link(&self.item.resolve(cx)?, &self.url, &self.role, &self.label)
                .await?,
        ))
    }
}

/// Link a repository revision.
#[cmd(write, view = done)]
pub struct AddLinkedRevision {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Revision
    pub revision: String,
    /// Repository (default repository if omitted)
    pub repository: Option<String>,
}

impl Run for AddLinkedRevision {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let repository = self.repository;
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_linked_revision(
                    &self.item.resolve(cx)?,
                    repository.as_deref(),
                    &self.revision,
                )
                .await?,
        ))
    }
}

/// Unlink a repository revision.
#[cmd(write, view = done)]
pub struct RemoveLinkedRevision {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Revision
    pub revision: String,
    /// Repository
    pub repository: Option<String>,
}

impl Run for RemoveLinkedRevision {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let repository = self.repository;
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_linked_revision(
                    &self.item.resolve(cx)?,
                    repository.as_deref(),
                    &self.revision,
                )
                .await?,
        ))
    }
}

/// Add a planning constraint.
#[cmd(write, view = done)]
pub struct AddPlanningConstraint {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Date
    pub date: String,
    /// Constraint id
    pub constraint: String,
}

impl Run for AddPlanningConstraint {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_planning_constraint(&self.item.resolve(cx)?, &self.date, &self.constraint)
                .await?,
        ))
    }
}

/// Remove a planning constraint.
#[cmd(write, view = done)]
pub struct RemovePlanningConstraint {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Date
    pub date: String,
    /// Constraint id
    pub constraint: String,
}

impl Run for RemovePlanningConstraint {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_planning_constraint(&self.item.resolve(cx)?, &self.date, &self.constraint)
                .await?,
        ))
    }
}

/// Assign a user.
#[cmd(write, view = done)]
pub struct AddAssignee {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
}

impl Run for AddAssignee {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_assignee(&self.item.resolve(cx)?, &self.user)
                .await?,
        ))
    }
}

/// Unassign a user.
#[cmd(write, view = done)]
pub struct RemoveAssignee {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
}

impl Run for RemoveAssignee {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_assignee(&self.item.resolve(cx)?, &self.user)
                .await?,
        ))
    }
}

/// Add a category.
#[cmd(write, view = done)]
pub struct AddCategory {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Category id
    pub category: String,
}

impl Run for AddCategory {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .add_category(&self.item.resolve(cx)?, &self.category)
                .await?,
        ))
    }
}

/// Remove a category.
#[cmd(write, view = done)]
pub struct RemoveCategory {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Category id
    pub category: String,
}

impl Run for RemoveCategory {
    type Output = Changed;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(Changed::from(
            cx.polarion
                .items()
                .remove_category(&self.item.resolve(cx)?, &self.category)
                .await?,
        ))
    }
}

/// Request an approval from a user.
#[cmd(write, view = done)]
pub struct AddApprovee {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
}

impl Run for AddApprovee {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .add_approvee(&self.item.resolve(cx)?, &self.user)
            .await?;
        Ok(Done)
    }
}

/// Remove an approver.
#[cmd(write, view = done)]
pub struct RemoveApprovee {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
}

impl Run for RemoveApprovee {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .remove_approvee(&self.item.resolve(cx)?, &self.user)
            .await?;
        Ok(Done)
    }
}

/// Set a user's approval status.
#[cmd(write, view = done)]
pub struct SetApproval {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
    /// e.g. `approved`, `disapproved`, `waiting`
    pub status: String,
}

impl Run for SetApproval {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .set_approval(&self.item.resolve(cx)?, &self.user, &self.status)
            .await?;
        Ok(Done)
    }
}

/// Attach a file.
#[cmd(write, view = done)]
pub struct AddAttachment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// File name
    pub file_name: String,
    /// Title
    pub title: String,
    /// Content
    pub data: Data,
}

impl Run for AddAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .add_attachment(
                &self.item.resolve(cx)?,
                &self.file_name,
                &self.title,
                &self.data.0,
            )
            .await?;
        Ok(Done)
    }
}

/// Change an attachment's file name, title or content.
#[cmd(write, view = done)]
pub struct UpdateAttachment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Attachment id
    pub attachment: String,
    /// File name
    pub file_name: Option<String>,
    /// Title
    pub title: Option<String>,
    /// Content
    pub data: Option<Data>,
}

impl Run for UpdateAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (file_name, title, data) = (self.file_name, self.title, self.data.map(|d| d.0));
        cx.polarion
            .items()
            .update_attachment(
                &self.item.resolve(cx)?,
                &self.attachment,
                file_name.as_deref(),
                title.as_deref(),
                data.as_deref(),
            )
            .await?;
        Ok(Done)
    }
}

/// Delete an attachment.
#[cmd(write, view = done)]
pub struct DeleteAttachment {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Attachment id
    pub attachment: String,
}

impl Run for DeleteAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .delete_attachment(&self.item.resolve(cx)?, &self.attachment)
            .await?;
        Ok(Done)
    }
}

/// Book time.
#[cmd(write, view = done)]
pub struct AddWorkRecord {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// User id
    pub user: String,
    /// Date (YYYY-MM-DD)
    pub date: String,
    /// Duration, e.g. `2h`
    pub time_spent: String,
    /// Work record type id (needed with a comment)
    pub r#type: Option<String>,
    /// Comment
    pub comment: Option<String>,
}

impl Run for AddWorkRecord {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let record = NewWorkRecord {
            user: self.user,
            date: self.date,
            time_spent: self.time_spent,
            kind: self.r#type,
            comment: self.comment,
        };
        cx.polarion
            .items()
            .add_work_record(&self.item.resolve(cx)?, &record)
            .await?;
        Ok(Done)
    }
}

/// Delete a work record.
#[cmd(write, view = done)]
pub struct DeleteWorkRecord {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Work record URI
    pub record: String,
}

impl Run for DeleteWorkRecord {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .delete_work_record(&self.item.resolve(cx)?, &self.record)
            .await?;
        Ok(Done)
    }
}

/// Set the title in a language.
#[cmd(write, view = done)]
pub struct SetLocalizedTitle {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Language id (default language if omitted)
    pub language: Option<String>,
    /// Title (clears if omitted)
    pub title: Option<String>,
}

impl Run for SetLocalizedTitle {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (language, title) = (self.language, self.title);
        cx.polarion
            .items()
            .set_localized_title(
                &self.item.resolve(cx)?,
                language.as_deref(),
                title.as_deref(),
            )
            .await?;
        Ok(Done)
    }
}

/// Set the description in a language.
#[cmd(write, view = done)]
pub struct SetLocalizedDescription {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Language id (default language if omitted)
    pub language: Option<String>,
    /// Description (clears if omitted)
    pub description: Option<RichText>,
}

impl Run for SetLocalizedDescription {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (language, description) = (self.language, self.description.map(polarion::Text::from));
        cx.polarion
            .items()
            .set_localized_description(
                &self.item.resolve(cx)?,
                language.as_deref(),
                description.as_ref(),
            )
            .await?;
        Ok(Done)
    }
}

/// Mark links to the work item suspect.
#[cmd(write, view = done)]
pub struct AutoSuspect {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for AutoSuspect {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .auto_suspect(&self.item.resolve(cx)?)
            .await?;
        Ok(Done)
    }
}

/// Assign by the project's rules.
#[cmd(write, view = done)]
pub struct AutoAssign {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for AutoAssign {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .items()
            .auto_assign(&self.item.resolve(cx)?)
            .await?;
        Ok(Done)
    }
}

// ---- reads

// ---- writes

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn id_prefixes_choose_the_linked_ids() {
        let ids = |list: &[&str]| work_item_ids(&strings(list)).map_err(|e| e.message);
        assert_eq!(ids(&[]), Ok(None));
        assert_eq!(ids(&[" ", ""]), Ok(None));
        assert_eq!(
            ids(&["MP", " OP ", "MY_PROJ2"]),
            Ok(Some(WorkItemIds::Prefixes(strings(&[
                "MP", "OP", "MY_PROJ2"
            ]))))
        );
        assert_eq!(ids(&["MP", "*"]), Ok(Some(WorkItemIds::Any)));
        assert_eq!(ids(&["none"]), Ok(Some(WorkItemIds::Off)));
        assert!(
            ids(&["none", "MP"])
                .unwrap_err()
                .contains("cannot be combined")
        );
        // Prefixes the conversion would never find refuse the command instead of linking nothing.
        for bad in ["mp", "Mp", "MP-", "M", "1P", "M P", "MP,", "_MP"] {
            let e = ids(&["OP", bad]).unwrap_err();
            assert!(
                e.starts_with(&format!(
                    "id-prefixes: `{bad}` is not a work item id prefix"
                )),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn markdown_options_come_from_the_arguments() {
        let read: DescriptionMarkdown = crate::parse_as(json!({"item": "P-1"})).unwrap();
        assert_eq!(read.options().unwrap(), MarkdownReadOptions::default());
        let read: DescriptionMarkdown =
            crate::parse_as(json!({"item": "P-1", "drop-html": true, "id-prefixes": "MP, OP"}))
                .unwrap();
        assert_eq!(
            read.options().unwrap(),
            MarkdownReadOptions {
                raw_html: RawHtml::Drop,
                work_items: Some(WorkItemIds::Prefixes(strings(&["MP", "OP"]))),
            }
        );
        let read: DescriptionMarkdown =
            crate::parse_as(json!({"item": "P-1", "id-prefixes": "mp"})).unwrap();
        assert!(read.options().is_err());

        assert_eq!(
            write_options(&[], false, false).unwrap(),
            MarkdownWriteOptions::default()
        );
        assert!(write_options(&strings(&["mp"]), false, false).is_err());
        assert_eq!(
            write_options(&strings(&["*"]), true, true).unwrap(),
            MarkdownWriteOptions {
                work_items: Some(WorkItemIds::Any),
                highlight: true,
                keep_orphans: true,
                allow_unrendered: true,
            }
        );
    }
}
