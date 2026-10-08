//! `activities.*`: activity streams.

use polarion::ActivityScope;
use polarion_commands_derive::{cmd, cmd_args};

use crate::{ACTIVITY_COLUMNS, CommandError, Context, Done, Result, RichText, Run};

/// Which activities: of a project, a project group or the whole repository, optionally filtered.
#[cmd_args]
pub struct ActivityFilter {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Project group URI instead of a project
    pub group: Option<String>,
    /// The whole repository instead of a project
    pub everywhere: bool,
    /// Lucene filter
    pub query: Option<String>,
    /// How many (default 50)
    pub count: Option<u32>,
}

impl ActivityFilter {
    fn scope<'s>(&'s self, project: &'s Option<String>) -> Result<ActivityScope<'s>> {
        Ok(if self.everywhere {
            ActivityScope::All
        } else if let Some(group) = self.group.as_deref() {
            ActivityScope::Group(group)
        } else {
            ActivityScope::Project(project.as_deref().ok_or_else(|| {
                CommandError::invalid("no scope: pass `project`, `group` or `everywhere`")
            })?)
        })
    }
}

/// Latest activities of a project, group or the whole repository.
#[cmd(view = table(ACTIVITY_COLUMNS))]
pub struct List {
    #[flatten]
    pub filter: ActivityFilter,
}

impl Run for List {
    type Output = Vec<polarion::soap::types::Activity>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let f = &self.filter;
        let project = cx.opt_project(f.project.as_deref());
        let count = f.count.unwrap_or(50);
        Ok(cx
            .polarion
            .activities()
            .list(f.scope(&project)?, f.query.as_deref(), count)
            .await?)
    }
}

/// Global ids of the latest activities.
#[cmd(view = lines)]
pub struct ListIds {
    #[flatten]
    pub filter: ActivityFilter,
}

impl Run for ListIds {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let f = &self.filter;
        let project = cx.opt_project(f.project.as_deref());
        let count = f.count.unwrap_or(50);
        Ok(cx
            .polarion
            .activities()
            .list_ids(f.scope(&project)?, f.query.as_deref(), count)
            .await?)
    }
}

/// One activity.
#[cmd(view = object)]
pub struct Get {
    /// Activity global id
    #[positional]
    pub id: String,
}

impl Run for Get {
    type Output = polarion::soap::types::Activity;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.activities().get(&self.id).await?)
    }
}

/// Activity sources.
#[cmd(view = object)]
pub struct Sources {}

impl Run for Sources {
    type Output = Vec<polarion::soap::types::ActivitySource>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.activities().sources().await?)
    }
}

/// Whether an activity accepts comments.
#[cmd(view = scalar)]
pub struct CanComment {
    /// Activity global id
    #[positional]
    pub id: String,
}

impl Run for CanComment {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.activities().can_comment(&self.id).await?)
    }
}

/// Whether you may comment on an activity.
#[cmd(view = scalar)]
pub struct CanCurrentUserComment {
    /// Activity global id
    #[positional]
    pub id: String,
}

impl Run for CanCurrentUserComment {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .activities()
            .can_current_user_comment(&self.id)
            .await?)
    }
}

/// Comment on an activity (needs POLARION_ALLOW_GLOBAL_WRITES)
#[cmd(write, view = done)]
pub struct AddComment {
    /// Activity global id
    #[positional]
    pub id: String,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
}

impl Run for AddComment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .activities()
            .add_comment(&self.id, &polarion::Text::from(self.text))
            .await?;
        Ok(Done)
    }
}
