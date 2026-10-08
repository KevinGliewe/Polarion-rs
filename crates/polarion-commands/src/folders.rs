//! `folders.*`: folders of documents and pages.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const GLOBAL: Param = Param::opt(
    "global",
    Kind::Boolean,
    "The global level instead of a project",
);

/// The project, or `None` for the global level.
fn scope(a: &Args) -> Result<Option<String>> {
    if a.bool("global")? {
        Ok(None)
    } else {
        a.project().map(Some)
    }
}

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "folders",
        "list",
        false,
        Output::Table(FOLDER_COLUMNS),
        "All folders",
        [PROJECT, GLOBAL],
        list
    ),
    command!(
        "folders",
        "roots",
        false,
        Output::Table(FOLDER_COLUMNS),
        "Top-level folders",
        [PROJECT, GLOBAL],
        roots
    ),
    command!(
        "folders",
        "children",
        false,
        Output::Table(FOLDER_COLUMNS),
        "Sub-folders of a folder",
        [
            Param::pos("parent", Kind::String, "Parent folder name"),
            PROJECT,
            GLOBAL
        ],
        children
    ),
    command!(
        "folders",
        "get",
        false,
        Output::Object,
        "One folder",
        [
            Param::pos("name", Kind::String, "Folder name"),
            PROJECT,
            GLOBAL
        ],
        get
    ),
    command!(
        "folders",
        "containing",
        false,
        Output::Object,
        "The folder containing an object",
        [URI],
        containing
    ),
    command!(
        "folders",
        "create",
        true,
        Output::Object,
        "Create a folder (global folders need POLARION_ALLOW_GLOBAL_WRITES)",
        [
            Param::pos("name", Kind::String, "Folder name (unique)"),
            Param::req("title", Kind::String, "Title"),
            Param::opt(
                "parent",
                Kind::String,
                "Parent folder name (top level if omitted)"
            ),
            PROJECT,
            GLOBAL,
        ],
        create
    ),
];

async fn list(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.folders().list(scope(a)?.as_deref()).await?)
}

async fn roots(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.folders().roots(scope(a)?.as_deref()).await?)
}

async fn children(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .folders()
        .children(scope(a)?.as_deref(), &a.str("parent")?)
        .await?)
}

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .folders()
        .get(scope(a)?.as_deref(), &a.str("name")?)
        .await?)
}

async fn containing(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.folders().containing(&a.str("uri")?).await?)
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let parent = a.opt_str("parent")?;
    out(p
        .folders()
        .create(
            scope(a)?.as_deref(),
            parent.as_deref(),
            &a.str("name")?,
            &a.str("title")?,
        )
        .await?)
}
