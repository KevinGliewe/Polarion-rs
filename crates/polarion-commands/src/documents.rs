//! `documents.*`: live documents (modules).

use polarion::soap::types::PdfProperties;
use polarion::{DocumentUpdate, NewDocument, ReuseDocument};
use polarion_commands_derive::cmd;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ACTION_COLUMNS, Context, DOCUMENT_COLUMNS, DOCUMENT_ITEM_COLUMNS, DocumentArg, Done, Download,
    FieldSelection, ItemArg, JsonObject, QueryArgs, REF_COLUMNS, Result, RichText, Run,
};

/// Document spaces of a project.
#[cmd(view = lines)]
pub struct Spaces {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Spaces {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .spaces(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Sub-folders of a location in the documents folder.
#[cmd(view = lines)]
pub struct SubFolders {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Location relative to the documents folder
    pub location: String,
}

impl Run for SubFolders {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .sub_folders(&cx.project(self.project.as_deref())?, &self.location)
            .await?)
    }
}

/// Documents of a project or one space (without bodies unless `full`)
#[cmd(view = table(DOCUMENT_COLUMNS))]
pub struct List {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Space id (`_default` is the default space)
    pub space: Option<String>,
    /// All fields incl. bodies (needs `space`)
    pub full: bool,
}

impl Run for List {
    type Output = Vec<polarion::Document>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (project, space) = (cx.project(self.project.as_deref())?, self.space);
        if self.full {
            let space =
                space.ok_or_else(|| crate::CommandError::invalid("`full` needs `space`"))?;
            return Ok(cx.polarion.documents().list_full(&project, &space).await?);
        }
        Ok(cx
            .polarion
            .documents()
            .list(&project, space.as_deref())
            .await?)
    }
}

/// References of the documents in one space.
#[cmd(view = table(REF_COLUMNS))]
pub struct Refs {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Space id
    pub space: String,
}

impl Run for Refs {
    type Output = Vec<polarion::DocumentRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .refs(&cx.project(self.project.as_deref())?, &self.space)
            .await?)
    }
}

/// Get a document (with `body`, or only `fields`)
#[cmd(view = object)]
pub struct Get {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Include the document body (Polarion HTML)
    pub body: bool,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
}

impl Run for Get {
    type Output = polarion::Document;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let document = self.document.resolve(cx)?;
        if let Some(fields) = FieldSelection::fields(self.fields.as_ref(), &[]) {
            return Ok(cx
                .polarion
                .documents()
                .get_by_ref(&document, Some(&fields))
                .await?);
        }
        match (&document.project, &document.space, &document.name) {
            (Some(project), Some(space), Some(name)) => Ok(cx
                .polarion
                .documents()
                .get(project, space, name, self.body)
                .await?),
            _ => Ok(cx.polarion.documents().get_by_ref(&document, None).await?),
        }
    }
}

/// Search documents with a Lucene query.
#[cmd(view = table(DOCUMENT_COLUMNS))]
pub struct Query {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Query {
    type Output = Vec<polarion::Document>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .query(&self.search.query(cx))
            .await?)
    }
}

/// Search documents, returning references.
#[cmd(view = table(REF_COLUMNS))]
pub struct QueryRefs {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for QueryRefs {
    type Output = Vec<polarion::DocumentRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .query_refs(&self.search.query(cx))
            .await?)
    }
}

/// Select documents with SQL.
#[cmd(view = table(DOCUMENT_COLUMNS))]
pub struct QuerySql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QuerySql {
    type Output = Vec<polarion::Document>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (fields, baseline) = (
            FieldSelection::fields(self.fields.as_ref(), &[]),
            self.baseline,
        );
        Ok(cx
            .polarion
            .documents()
            .query_sql(&self.sql, fields.as_ref(), baseline.as_deref())
            .await?)
    }
}

/// Select document references with SQL.
#[cmd(view = table(REF_COLUMNS))]
pub struct QueryRefsSql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QueryRefsSql {
    type Output = Vec<polarion::DocumentRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let baseline = self.baseline;
        Ok(cx
            .polarion
            .documents()
            .query_refs_sql(&self.sql, baseline.as_deref())
            .await?)
    }
}

/// Work items of a document in document order.
#[cmd(view = table(DOCUMENT_ITEM_COLUMNS))]
pub struct WorkItems {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Parent work item (top level if omitted)
    pub parent: Option<ItemArg>,
    /// Whole subtree (default true)
    pub deep: Option<bool>,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
}

impl Run for WorkItems {
    type Output = Vec<polarion::WorkItem>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let document = self.document.resolve(cx)?;
        let (parent, deep, fields) = (
            self.parent.as_ref().map(|i| i.resolve(cx)).transpose()?,
            self.deep.unwrap_or(true),
            FieldSelection::fields(self.fields.as_ref(), &self.custom).unwrap_or_default(),
        );
        if parent.is_none() && deep {
            return Ok(cx
                .polarion
                .documents()
                .work_items(&document, &fields)
                .await?);
        }
        Ok(cx
            .polarion
            .documents()
            .work_items_under(&document, parent.as_ref(), deep, &fields)
            .await?)
    }
}

/// References of the work items of a document.
#[cmd(view = table(REF_COLUMNS))]
pub struct WorkItemRefs {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Parent work item (top level if omitted)
    pub parent: Option<ItemArg>,
    /// Whole subtree (default true)
    pub deep: Option<bool>,
}

impl Run for WorkItemRefs {
    type Output = Vec<polarion::ItemRef>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let parent = self.parent.as_ref().map(|i| i.resolve(cx)).transpose()?;
        Ok(cx
            .polarion
            .documents()
            .work_item_refs(
                &self.document.resolve(cx)?,
                parent.as_ref(),
                self.deep.unwrap_or(true),
            )
            .await?)
    }
}

/// Workflow actions available for a document.
#[cmd(view = table(ACTION_COLUMNS))]
pub struct Actions {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
}

impl Run for Actions {
    type Output = Vec<polarion::WorkflowAction>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .actions(&self.document.resolve(cx)?)
            .await?)
    }
}

/// Page orientation of a PDF export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl Orientation {
    fn as_str(self) -> &'static str {
        match self {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
    }
}

/// Export a document as PDF.
#[cmd(view = bytes)]
pub struct ExportPdf {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Paper size (default A4)
    pub paper_size: Option<String>,
    /// Page orientation (default portrait)
    pub orientation: Option<Orientation>,
    /// Fit to page width
    pub fit_to_page_width: bool,
    /// Generate bookmarks
    pub bookmarks: bool,
    /// Include header and footer
    pub header_footer: bool,
    /// Repeat table headers
    pub repeat_table_headers: bool,
}

impl Run for ExportPdf {
    type Output = Download;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let options = PdfProperties {
            paper_size: self.paper_size.unwrap_or_else(|| "A4".into()),
            orientation: self.orientation.unwrap_or_default().as_str().to_owned(),
            fit_to_page_width: self.fit_to_page_width,
            generate_bookmarks: self.bookmarks,
            include_header_footer: self.header_footer,
            repeat_table_headers: self.repeat_table_headers,
        };
        Ok(Download(
            cx.polarion
                .documents()
                .export_pdf(&self.document.resolve(cx)?, &options)
                .await?,
        ))
    }
}

/// Create a live document.
#[cmd(write, view = object)]
pub struct Create {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Space id (`_default` is the default space)
    pub space: Option<String>,
    /// Document name
    pub name: String,
    /// Title
    pub title: Option<String>,
    /// Allowed work item types
    pub allowed_types: Vec<String>,
    /// Link role structuring the document (default `parent`)
    pub structure_link_role: Option<String>,
    /// Initial content (HTML)
    pub content: Option<String>,
    /// Create an old-style module (createModule)
    pub legacy_module: bool,
    /// Old-style modules: links go from parent to child
    pub parent_to_child: bool,
}

/// A new or reused document: its reference and URI.
#[derive(Debug, Clone, Serialize)]
pub struct CreatedDocument {
    pub document: polarion::DocumentRef,
    pub uri: String,
}

impl From<polarion::DocumentRef> for CreatedDocument {
    fn from(document: polarion::DocumentRef) -> Self {
        Self {
            uri: document.uri.clone(),
            document,
        }
    }
}

impl Run for Create {
    type Output = CreatedDocument;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let new = NewDocument {
            project: cx.project(self.project.as_deref())?,
            space: self.space,
            name: self.name,
            title: self.title,
            allowed_types: self.allowed_types,
            structure_link_role: self.structure_link_role.unwrap_or_else(|| "parent".into()),
            body: self.content,
        };
        let created = if self.legacy_module {
            cx.polarion
                .documents()
                .create_module(&new, self.parent_to_child)
                .await?
        } else {
            cx.polarion.documents().create(&new).await?
        };
        Ok(created.into())
    }
}

/// Change fields of a document.
#[cmd(write, view = done)]
pub struct Update {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Title
    pub title: Option<String>,
    /// Document type id
    pub r#type: Option<String>,
    /// Status id
    pub status: Option<String>,
    /// The whole body
    pub content: Option<RichText>,
    /// Structure link role
    pub structure_link_role: Option<String>,
    /// Use outline numbering
    pub outline_numbering: Option<bool>,
    /// Custom field values (typed from JSON; use {"enum": id} for enums)
    pub custom_fields: Option<JsonObject>,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let update = DocumentUpdate {
            title: self.title,
            kind: self.r#type,
            status: self.status,
            body: self.content.map(polarion::Text::from),
            structure_link_role: self.structure_link_role,
            uses_outline_numbering: self.outline_numbering,
            custom_fields: crate::values::custom_values(
                &self.custom_fields.unwrap_or_default().0,
                &[],
            )?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
        };
        cx.polarion
            .documents()
            .update(&self.document.resolve(cx)?, &update)
            .await?;
        Ok(Done)
    }
}

/// Delete a document.
#[cmd(write, view = done)]
pub struct Delete {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
}

impl Run for Delete {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .documents()
            .delete(&self.document.resolve(cx)?)
            .await?;
        Ok(Done)
    }
}

/// Set the first title heading in the content.
#[cmd(write, view = scalar)]
pub struct UpdateTitleHeading {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Title
    pub title: Option<String>,
}

impl Run for UpdateTitleHeading {
    type Output = Option<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let title = self.title;
        Ok(cx
            .polarion
            .documents()
            .update_title_heading(&self.document.resolve(cx)?, title.as_deref())
            .await?)
    }
}

/// Create a new document from an existing one.
#[cmd(write, view = object)]
pub struct Reuse {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Target project (default: the default project)
    pub target_project: Option<String>,
    /// Target space (default space if omitted)
    pub target_space: Option<String>,
    /// Target document name
    pub target_name: String,
    /// Target document title
    pub target_title: Option<String>,
    /// Also change the first title heading to the title
    pub update_title: bool,
    /// Role of links from reused items to their sources
    pub link_role: Option<String>,
    /// Fields kept in sync with the source
    pub derived_fields: Vec<String>,
    /// Old-style reuse (reuseModule)
    pub legacy_module: bool,
    /// Old-style reuse: fields to copy
    pub fields: Vec<String>,
    /// Old-style reuse: fields not to copy
    pub except_fields: Vec<String>,
}

impl Run for Reuse {
    type Output = CreatedDocument;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let target = ReuseDocument {
            project: match self.target_project {
                Some(project) => project,
                None => cx.project(None)?,
            },
            space: self.target_space,
            name: self.target_name,
            title: self.target_title,
            update_title_in_document: self.update_title,
            link_role: self.link_role,
            derived_fields: self.derived_fields,
        };
        let source = self.document.resolve(cx)?;
        let created = if self.legacy_module {
            cx.polarion
                .documents()
                .reuse_module(&source, &target, &self.fields, &self.except_fields)
                .await?
        } else {
            cx.polarion.documents().reuse(&source, &target).await?
        };
        Ok(created.into())
    }
}

/// Update a derived document to a revision of its source.
#[cmd(write, view = done)]
pub struct UpdateDerived {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Revision (HEAD if omitted)
    pub revision: Option<String>,
    /// Mark changed links suspect
    pub auto_suspect: bool,
    /// Old-style module (updateDerivedModule)
    pub legacy_module: bool,
}

impl Run for UpdateDerived {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (document, revision) = (self.document.resolve(cx)?, self.revision);
        if self.legacy_module {
            cx.polarion
                .documents()
                .update_derived_module(&document, revision.as_deref())
                .await?;
        } else {
            cx.polarion
                .documents()
                .update_derived(&document, revision.as_deref(), self.auto_suspect)
                .await?;
        }
        Ok(Done)
    }
}

/// Comment on a document, optionally about one of its work items.
#[cmd(write, view = scalar)]
pub struct AddComment {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
    /// Work item the comment refers to
    pub about: Option<ItemArg>,
}

impl Run for AddComment {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let about = self.about.as_ref().map(|i| i.resolve(cx)).transpose()?;
        Ok(cx
            .polarion
            .documents()
            .add_comment(
                &self.document.resolve(cx)?,
                about.as_ref(),
                &polarion::Text::from(self.text),
            )
            .await?)
    }
}

/// Reply to a document comment.
#[cmd(write, view = scalar)]
pub struct ReplyToComment {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
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
            .documents()
            .reply_to_comment(
                &self.document.resolve(cx)?,
                &self.comment,
                &polarion::Text::from(self.text),
            )
            .await?)
    }
}

/// Perform a workflow action on a document.
#[cmd(write, view = done)]
pub struct PerformAction {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Action id
    pub action: i32,
}

impl Run for PerformAction {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .documents()
            .perform_action(&self.document.resolve(cx)?, self.action)
            .await?;
        Ok(Done)
    }
}

/// Move a document to a status via the matching workflow action.
#[cmd(write, view = object)]
pub struct Transition {
    /// Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI
    #[positional]
    pub document: DocumentArg,
    /// Target status id
    pub status: String,
}

impl Run for Transition {
    type Output = polarion::WorkflowAction;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .documents()
            .transition(&self.document.resolve(cx)?, &self.status)
            .await?)
    }
}
