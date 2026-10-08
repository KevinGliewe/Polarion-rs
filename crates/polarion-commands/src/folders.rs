//! `folders.*`: folders of documents and pages.

use polarion_commands_derive::cmd;

use crate::{Context, FOLDER_COLUMNS, Result, Run};

/// All folders.
#[cmd(view = table(FOLDER_COLUMNS))]
pub struct List {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// The global level instead of a project
    pub global: bool,
}

impl Run for List {
    type Output = Vec<polarion::soap::types::Folder>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .folders()
            .list(scope(cx, self.project.as_deref(), self.global)?.as_deref())
            .await?)
    }
}

/// Top-level folders.
#[cmd(view = table(FOLDER_COLUMNS))]
pub struct Roots {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// The global level instead of a project
    pub global: bool,
}

impl Run for Roots {
    type Output = Vec<polarion::soap::types::Folder>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .folders()
            .roots(scope(cx, self.project.as_deref(), self.global)?.as_deref())
            .await?)
    }
}

/// Sub-folders of a folder.
#[cmd(view = table(FOLDER_COLUMNS))]
pub struct Children {
    /// Parent folder name
    #[positional]
    pub parent: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// The global level instead of a project
    pub global: bool,
}

impl Run for Children {
    type Output = Vec<polarion::soap::types::Folder>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .folders()
            .children(
                scope(cx, self.project.as_deref(), self.global)?.as_deref(),
                &self.parent,
            )
            .await?)
    }
}

/// One folder.
#[cmd(view = object)]
pub struct Get {
    /// Folder name
    #[positional]
    pub name: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// The global level instead of a project
    pub global: bool,
}

impl Run for Get {
    type Output = polarion::soap::types::Folder;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .folders()
            .get(
                scope(cx, self.project.as_deref(), self.global)?.as_deref(),
                &self.name,
            )
            .await?)
    }
}

/// The folder containing an object.
#[cmd(view = object)]
pub struct Containing {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for Containing {
    type Output = polarion::soap::types::Folder;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.folders().containing(&self.uri).await?)
    }
}

/// Create a folder (global folders need POLARION_ALLOW_GLOBAL_WRITES)
#[cmd(write, view = object)]
pub struct Create {
    /// Folder name (unique)
    #[positional]
    pub name: String,
    /// Title
    pub title: String,
    /// Parent folder name (top level if omitted)
    pub parent: Option<String>,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// The global level instead of a project
    pub global: bool,
}

impl Run for Create {
    type Output = polarion::soap::types::Folder;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let parent = self.parent;
        Ok(cx
            .polarion
            .folders()
            .create(
                scope(cx, self.project.as_deref(), self.global)?.as_deref(),
                parent.as_deref(),
                &self.name,
                &self.title,
            )
            .await?)
    }
}

/// The project's folders, or the global ones.
fn scope(cx: &Context<'_>, project: Option<&str>, global: bool) -> Result<Option<String>> {
    if global {
        Ok(None)
    } else {
        cx.project(project).map(Some)
    }
}
