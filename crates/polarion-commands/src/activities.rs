//! `activities.*`: activity streams.

use polarion::{ActivityScope, Polarion};
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const ID: Param = Param::pos("id", Kind::String, "Activity global id");
const SCOPE: [Param; 4] = [
    PROJECT,
    Param::opt(
        "group",
        Kind::String,
        "Project group URI instead of a project",
    ),
    Param::opt(
        "everywhere",
        Kind::Boolean,
        "The whole repository instead of a project",
    ),
    Param::opt("query", Kind::String, "Lucene filter"),
];

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "activities",
        "list",
        false,
        Output::Table(ACTIVITY_COLUMNS),
        "Latest activities of a project, group or the whole repository",
        [
            SCOPE[0],
            SCOPE[1],
            SCOPE[2],
            SCOPE[3],
            Param::opt("count", Kind::Integer, "How many (default 50)")
        ],
        list
    ),
    command!(
        "activities",
        "list-ids",
        false,
        Output::Lines,
        "Global ids of the latest activities",
        [
            SCOPE[0],
            SCOPE[1],
            SCOPE[2],
            SCOPE[3],
            Param::opt("count", Kind::Integer, "How many (default 50)")
        ],
        list_ids
    ),
    command!(
        "activities",
        "get",
        false,
        Output::Object,
        "One activity",
        [ID],
        get
    ),
    command!(
        "activities",
        "sources",
        false,
        Output::Object,
        "Activity sources",
        [],
        sources
    ),
    command!(
        "activities",
        "can-comment",
        false,
        Output::Scalar,
        "Whether an activity accepts comments",
        [ID],
        can_comment
    ),
    command!(
        "activities",
        "can-current-user-comment",
        false,
        Output::Scalar,
        "Whether you may comment on an activity",
        [ID],
        can_current_user_comment
    ),
    command!(
        "activities",
        "add-comment",
        true,
        Output::Done,
        "Comment on an activity (needs POLARION_ALLOW_GLOBAL_WRITES)",
        [ID, TEXT],
        add_comment
    ),
];

/// Runs `f` with the scope selected by the arguments.
async fn scoped(p: &Polarion, a: &Args, ids: bool) -> Result<Value> {
    let query = a.opt_str("query")?;
    let count = a.opt_u32("count")?.unwrap_or(50);
    let (group, project) = (a.opt_str("group")?, a.opt_project()?);
    let scope = if a.bool("everywhere")? {
        ActivityScope::All
    } else if let Some(group) = group.as_deref() {
        ActivityScope::Group(group)
    } else {
        ActivityScope::Project(project.as_deref().ok_or_else(|| {
            crate::CommandError::invalid("no scope: pass `project`, `group` or `everywhere`")
        })?)
    };
    if ids {
        out(p
            .activities()
            .list_ids(scope, query.as_deref(), count)
            .await?)
    } else {
        out(p.activities().list(scope, query.as_deref(), count).await?)
    }
}

async fn list(p: &Polarion, a: &Args) -> Result<Value> {
    scoped(p, a, false).await
}

async fn list_ids(p: &Polarion, a: &Args) -> Result<Value> {
    scoped(p, a, true).await
}

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.activities().get(&a.str("id")?).await?)
}

async fn sources(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.activities().sources().await?)
}

async fn can_comment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.activities().can_comment(&a.str("id")?).await?)
}

async fn can_current_user_comment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .activities()
        .can_current_user_comment(&a.str("id")?)
        .await?)
}

async fn add_comment(p: &Polarion, a: &Args) -> Result<Value> {
    p.activities()
        .add_comment(&a.str("id")?, &a.text("text")?)
        .await?;
    Ok(Value::Null)
}
