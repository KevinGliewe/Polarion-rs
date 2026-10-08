//! `projects.*`: projects and project groups.

use polarion_commands_derive::cmd;

use crate::{Context, GROUP_COLUMNS, PROJECT_COLUMNS, Result, Run};

/// A project (default: the default project)
#[cmd(view = object)]
pub struct Get {
    /// Project id
    #[positional]
    pub id: Option<String>,
}

impl Run for Get {
    type Output = polarion::Project;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let id = match self.id {
            Some(id) => id,
            None => cx.project(None)?,
        };
        Ok(cx.polarion.projects().get(&id).await?)
    }
}

/// A project by URI.
#[cmd(view = object)]
pub struct GetByUri {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for GetByUri {
    type Output = polarion::Project;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().get_by_uri(&self.uri).await?)
    }
}

/// The project at a repository location.
#[cmd(view = object)]
pub struct AtLocation {
    /// Repository location
    #[positional]
    pub location: String,
}

impl Run for AtLocation {
    type Output = polarion::Project;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().at_location(&self.location).await?)
    }
}

/// All projects you can see (walks the group tree)
#[cmd(view = table(PROJECT_COLUMNS))]
pub struct List {}

impl Run for List {
    type Output = Vec<polarion::Project>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().list().await?)
    }
}

/// Projects in a group (with `deep`, also in subgroups)
#[cmd(view = table(PROJECT_COLUMNS))]
pub struct InGroup {
    /// Project group URI
    #[positional]
    pub group: String,
    /// Include subgroups
    pub deep: bool,
}

impl Run for InGroup {
    type Output = Vec<polarion::Project>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .projects()
            .in_group(&self.group, self.deep)
            .await?)
    }
}

/// The root project group.
#[cmd(view = object)]
pub struct RootGroup {}

impl Run for RootGroup {
    type Output = polarion::ProjectGroup;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().root_group().await?)
    }
}

/// A project group by URI.
#[cmd(view = object)]
pub struct Group {
    /// Project group URI
    #[positional]
    pub group: String,
}

impl Run for Group {
    type Output = polarion::ProjectGroup;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().group(&self.group).await?)
    }
}

/// The project group at a location.
#[cmd(view = object)]
pub struct GroupAt {
    /// Repository location
    #[positional]
    pub location: String,
}

impl Run for GroupAt {
    type Output = polarion::ProjectGroup;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().group_at(&self.location).await?)
    }
}

/// Subgroups of a group.
#[cmd(view = table(GROUP_COLUMNS))]
pub struct GroupsIn {
    /// Project group URI
    #[positional]
    pub group: String,
}

impl Run for GroupsIn {
    type Output = Vec<polarion::ProjectGroup>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.projects().groups_in(&self.group).await?)
    }
}
