//! `documents.*`: live documents (modules).

use polarion::soap::types::PdfProperties;
use polarion::{DocumentUpdate, NewDocument, Polarion, ReuseDocument};
use serde_json::{Value, json};

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{bytes, command, out};

const SPACE: Param = Param::opt(
    "space",
    Kind::String,
    "Space id (`_default` is the default space)",
);
const BODY: Param = Param::opt(
    "body",
    Kind::Boolean,
    "Include the document body (Polarion HTML)",
);
const PARENT: Param = Param::opt(
    "parent",
    Kind::Item,
    "Parent work item (top level if omitted)",
);
const DEEP: Param = Param::opt("deep", Kind::Boolean, "Whole subtree (default true)");

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "documents",
        "spaces",
        false,
        Output::Lines,
        "Document spaces of a project",
        [PROJECT],
        spaces
    ),
    command!(
        "documents",
        "sub-folders",
        false,
        Output::Lines,
        "Sub-folders of a location in the documents folder",
        [
            PROJECT,
            Param::req(
                "location",
                Kind::String,
                "Location relative to the documents folder"
            )
        ],
        sub_folders
    ),
    command!(
        "documents",
        "list",
        false,
        Output::Table(DOCUMENT_COLUMNS),
        "Documents of a project or one space (without bodies unless `full`)",
        [
            PROJECT,
            SPACE,
            Param::opt(
                "full",
                Kind::Boolean,
                "All fields incl. bodies (needs `space`)"
            )
        ],
        list
    ),
    command!(
        "documents",
        "refs",
        false,
        Output::Table(REF_COLUMNS),
        "References of the documents in one space",
        [PROJECT, Param::req("space", Kind::String, "Space id")],
        refs
    ),
    command!(
        "documents",
        "get",
        false,
        Output::Object,
        "Get a document (with `body`, or only `fields`)",
        [DOCUMENT, BODY, FIELDS],
        get
    ),
    command!(
        "documents",
        "query",
        false,
        Output::Table(DOCUMENT_COLUMNS),
        "Search documents with a Lucene query",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS, BASELINE],
        query
    ),
    command!(
        "documents",
        "query-refs",
        false,
        Output::Table(REF_COLUMNS),
        "Search documents, returning references",
        [QUERY, PROJECT, SORT, LIMIT, ALL, BASELINE],
        query_refs
    ),
    command!(
        "documents",
        "query-sql",
        false,
        Output::Table(DOCUMENT_COLUMNS),
        "Select documents with SQL",
        [SQL, FIELDS, BASELINE],
        query_sql
    ),
    command!(
        "documents",
        "query-refs-sql",
        false,
        Output::Table(REF_COLUMNS),
        "Select document references with SQL",
        [SQL, BASELINE],
        query_refs_sql
    ),
    command!(
        "documents",
        "work-items",
        false,
        Output::Table(DOCUMENT_ITEM_COLUMNS),
        "Work items of a document in document order",
        [DOCUMENT, PARENT, DEEP, FIELDS, CUSTOM],
        work_items
    ),
    command!(
        "documents",
        "work-item-refs",
        false,
        Output::Table(REF_COLUMNS),
        "References of the work items of a document",
        [DOCUMENT, PARENT, DEEP],
        work_item_refs
    ),
    command!(
        "documents",
        "actions",
        false,
        Output::Table(ACTION_COLUMNS),
        "Workflow actions available for a document",
        [DOCUMENT],
        actions
    ),
    command!(
        "documents",
        "export-pdf",
        false,
        Output::Bytes,
        "Export a document as PDF",
        [
            DOCUMENT,
            Param::opt("paper-size", Kind::String, "Paper size (default A4)"),
            Param::opt(
                "orientation",
                Kind::String,
                "portrait or landscape (default portrait)"
            ),
            Param::opt("fit-to-page-width", Kind::Boolean, "Fit to page width"),
            Param::opt("bookmarks", Kind::Boolean, "Generate bookmarks"),
            Param::opt("header-footer", Kind::Boolean, "Include header and footer"),
            Param::opt(
                "repeat-table-headers",
                Kind::Boolean,
                "Repeat table headers"
            ),
        ],
        export_pdf
    ),
    command!(
        "documents",
        "create",
        true,
        Output::Object,
        "Create a live document",
        [
            PROJECT,
            SPACE,
            Param::req("name", Kind::String, "Document name"),
            Param::opt("title", Kind::String, "Title"),
            Param::opt("allowed-types", Kind::StringList, "Allowed work item types"),
            Param::opt(
                "structure-link-role",
                Kind::String,
                "Link role structuring the document (default `parent`)"
            ),
            Param::opt("content", Kind::String, "Initial content (HTML)"),
            Param::opt(
                "legacy-module",
                Kind::Boolean,
                "Create an old-style module (createModule)"
            ),
            Param::opt(
                "parent-to-child",
                Kind::Boolean,
                "Old-style modules: links go from parent to child"
            ),
        ],
        create
    ),
    command!(
        "documents",
        "update",
        true,
        Output::Done,
        "Change fields of a document",
        [
            DOCUMENT,
            Param::opt("title", Kind::String, "Title"),
            Param::opt("type", Kind::String, "Document type id"),
            Param::opt("status", Kind::String, "Status id"),
            Param::opt("content", Kind::Text, "The whole body"),
            Param::opt("structure-link-role", Kind::String, "Structure link role"),
            Param::opt("outline-numbering", Kind::Boolean, "Use outline numbering"),
            Param::opt(
                "custom-fields",
                Kind::Json,
                "Custom field values (typed from JSON; use {\"enum\": id} for enums)"
            ),
        ],
        update
    ),
    command!(
        "documents",
        "delete",
        true,
        Output::Done,
        "Delete a document",
        [DOCUMENT],
        delete
    ),
    command!(
        "documents",
        "update-title-heading",
        true,
        Output::Scalar,
        "Set the first title heading in the content",
        [DOCUMENT, Param::opt("title", Kind::String, "Title")],
        update_title_heading
    ),
    command!(
        "documents",
        "reuse",
        true,
        Output::Object,
        "Create a new document from an existing one",
        [
            DOCUMENT,
            Param::opt(
                "target-project",
                Kind::String,
                "Target project (default: the default project)"
            ),
            Param::opt(
                "target-space",
                Kind::String,
                "Target space (default space if omitted)"
            ),
            Param::req("target-name", Kind::String, "Target document name"),
            Param::opt("target-title", Kind::String, "Target document title"),
            Param::opt(
                "update-title",
                Kind::Boolean,
                "Also change the first title heading to the title"
            ),
            Param::opt(
                "link-role",
                Kind::String,
                "Role of links from reused items to their sources"
            ),
            Param::opt(
                "derived-fields",
                Kind::StringList,
                "Fields kept in sync with the source"
            ),
            Param::opt(
                "legacy-module",
                Kind::Boolean,
                "Old-style reuse (reuseModule)"
            ),
            Param::opt(
                "fields",
                Kind::StringList,
                "Old-style reuse: fields to copy"
            ),
            Param::opt(
                "except-fields",
                Kind::StringList,
                "Old-style reuse: fields not to copy"
            )
        ],
        reuse
    ),
    command!(
        "documents",
        "update-derived",
        true,
        Output::Done,
        "Update a derived document to a revision of its source",
        [
            DOCUMENT,
            Param::opt("revision", Kind::String, "Revision (HEAD if omitted)"),
            Param::opt("auto-suspect", Kind::Boolean, "Mark changed links suspect"),
            Param::opt(
                "legacy-module",
                Kind::Boolean,
                "Old-style module (updateDerivedModule)"
            )
        ],
        update_derived
    ),
    command!(
        "documents",
        "add-comment",
        true,
        Output::Scalar,
        "Comment on a document, optionally about one of its work items",
        [
            DOCUMENT,
            TEXT,
            Param::opt("about", Kind::Item, "Work item the comment refers to")
        ],
        add_comment
    ),
    command!(
        "documents",
        "reply-to-comment",
        true,
        Output::Scalar,
        "Reply to a document comment",
        [
            DOCUMENT,
            Param::req("comment", Kind::String, "Comment URI"),
            TEXT
        ],
        reply_to_comment
    ),
    command!(
        "documents",
        "perform-action",
        true,
        Output::Done,
        "Perform a workflow action on a document",
        [DOCUMENT, Param::req("action", Kind::Integer, "Action id")],
        perform_action
    ),
    command!(
        "documents",
        "transition",
        true,
        Output::Object,
        "Move a document to a status via the matching workflow action",
        [
            DOCUMENT,
            Param::req("status", Kind::String, "Target status id")
        ],
        transition
    ),
];

async fn spaces(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.documents().spaces(&a.project()?).await?)
}

async fn sub_folders(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .documents()
        .sub_folders(&a.project()?, &a.str("location")?)
        .await?)
}

async fn list(p: &Polarion, a: &Args) -> Result<Value> {
    let (project, space) = (a.project()?, a.opt_str("space")?);
    if a.bool("full")? {
        let space = space.ok_or_else(|| crate::CommandError::invalid("`full` needs `space`"))?;
        return out(p.documents().list_full(&project, &space).await?);
    }
    out(p.documents().list(&project, space.as_deref()).await?)
}

async fn refs(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.documents().refs(&a.project()?, &a.str("space")?).await?)
}

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let document = a.document("document")?;
    if let Some(fields) = a.opt_fields()? {
        return out(p.documents().get_by_ref(&document, Some(&fields)).await?);
    }
    match (&document.project, &document.space, &document.name) {
        (Some(project), Some(space), Some(name)) => out(p
            .documents()
            .get(project, space, name, a.bool("body")?)
            .await?),
        _ => out(p.documents().get_by_ref(&document, None).await?),
    }
}

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.documents().query(&a.query()?).await?)
}

async fn query_refs(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.documents().query_refs(&a.query()?).await?)
}

async fn query_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let (fields, baseline) = (a.opt_fields()?, a.opt_str("baseline")?);
    out(p
        .documents()
        .query_sql(&a.str("sql")?, fields.as_ref(), baseline.as_deref())
        .await?)
}

async fn query_refs_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let baseline = a.opt_str("baseline")?;
    out(p
        .documents()
        .query_refs_sql(&a.str("sql")?, baseline.as_deref())
        .await?)
}

async fn work_items(p: &Polarion, a: &Args) -> Result<Value> {
    let document = a.document("document")?;
    let (parent, deep, fields) = (
        a.opt_item("parent")?,
        a.opt_bool("deep")?.unwrap_or(true),
        a.fields()?,
    );
    if parent.is_none() && deep {
        return out(p.documents().work_items(&document, &fields).await?);
    }
    out(p
        .documents()
        .work_items_under(&document, parent.as_ref(), deep, &fields)
        .await?)
}

async fn work_item_refs(p: &Polarion, a: &Args) -> Result<Value> {
    let parent = a.opt_item("parent")?;
    out(p
        .documents()
        .work_item_refs(
            &a.document("document")?,
            parent.as_ref(),
            a.opt_bool("deep")?.unwrap_or(true),
        )
        .await?)
}

async fn actions(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.documents().actions(&a.document("document")?).await?)
}

async fn export_pdf(p: &Polarion, a: &Args) -> Result<Value> {
    let options = PdfProperties {
        paper_size: a.opt_str("paper-size")?.unwrap_or_else(|| "A4".into()),
        orientation: a
            .opt_str("orientation")?
            .unwrap_or_else(|| "portrait".into()),
        fit_to_page_width: a.bool("fit-to-page-width")?,
        generate_bookmarks: a.bool("bookmarks")?,
        include_header_footer: a.bool("header-footer")?,
        repeat_table_headers: a.bool("repeat-table-headers")?,
    };
    bytes(
        &p.documents()
            .export_pdf(&a.document("document")?, &options)
            .await?,
    )
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let new = NewDocument {
        project: a.project()?,
        space: a.opt_str("space")?,
        name: a.str("name")?,
        title: a.opt_str("title")?,
        allowed_types: a.list("allowed-types")?,
        structure_link_role: a
            .opt_str("structure-link-role")?
            .unwrap_or_else(|| "parent".into()),
        body: a.opt_str("content")?,
    };
    let created = if a.bool("legacy-module")? {
        p.documents()
            .create_module(&new, a.bool("parent-to-child")?)
            .await?
    } else {
        p.documents().create(&new).await?
    };
    out(json!({"document": created, "uri": created.uri}))
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let update = DocumentUpdate {
        title: a.opt_str("title")?,
        kind: a.opt_str("type")?,
        status: a.opt_str("status")?,
        body: a.opt_text("content")?,
        structure_link_role: a.opt_str("structure-link-role")?,
        uses_outline_numbering: a.opt_bool("outline-numbering")?,
        custom_fields: a
            .custom_values("custom-fields", &[])?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
    };
    p.documents()
        .update(&a.document("document")?, &update)
        .await?;
    Ok(Value::Null)
}

async fn delete(p: &Polarion, a: &Args) -> Result<Value> {
    p.documents().delete(&a.document("document")?).await?;
    Ok(Value::Null)
}

async fn update_title_heading(p: &Polarion, a: &Args) -> Result<Value> {
    let title = a.opt_str("title")?;
    out(p
        .documents()
        .update_title_heading(&a.document("document")?, title.as_deref())
        .await?)
}

async fn reuse(p: &Polarion, a: &Args) -> Result<Value> {
    let target = ReuseDocument {
        project: match a.opt_str("target-project")? {
            Some(project) => project,
            None => a.project()?,
        },
        space: a.opt_str("target-space")?,
        name: a.str("target-name")?,
        title: a.opt_str("target-title")?,
        update_title_in_document: a.bool("update-title")?,
        link_role: a.opt_str("link-role")?,
        derived_fields: a.list("derived-fields")?,
    };
    let source = a.document("document")?;
    let created = if a.bool("legacy-module")? {
        p.documents()
            .reuse_module(
                &source,
                &target,
                &a.list("fields")?,
                &a.list("except-fields")?,
            )
            .await?
    } else {
        p.documents().reuse(&source, &target).await?
    };
    out(json!({"document": created, "uri": created.uri}))
}

async fn update_derived(p: &Polarion, a: &Args) -> Result<Value> {
    let (document, revision) = (a.document("document")?, a.opt_str("revision")?);
    if a.bool("legacy-module")? {
        p.documents()
            .update_derived_module(&document, revision.as_deref())
            .await?;
    } else {
        p.documents()
            .update_derived(&document, revision.as_deref(), a.bool("auto-suspect")?)
            .await?;
    }
    Ok(Value::Null)
}

async fn add_comment(p: &Polarion, a: &Args) -> Result<Value> {
    let about = a.opt_item("about")?;
    out(p
        .documents()
        .add_comment(&a.document("document")?, about.as_ref(), &a.text("text")?)
        .await?)
}

async fn reply_to_comment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .documents()
        .reply_to_comment(
            &a.document("document")?,
            &a.str("comment")?,
            &a.text("text")?,
        )
        .await?)
}

async fn perform_action(p: &Polarion, a: &Args) -> Result<Value> {
    p.documents()
        .perform_action(&a.document("document")?, a.i32("action")?)
        .await?;
    Ok(Value::Null)
}

async fn transition(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .documents()
        .transition(&a.document("document")?, &a.str("status")?)
        .await?)
}
