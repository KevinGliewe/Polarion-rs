//! `projects.*`: projects and project groups.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const GROUP: Param = Param::pos("group", Kind::String, "Project group URI");
const LOCATION: Param = Param::pos("location", Kind::String, "Repository location");

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "projects",
        "get",
        false,
        Output::Object,
        "A project (default: the default project)",
        [Param::pos_opt("id", Kind::String, "Project id")],
        get
    ),
    command!(
        "projects",
        "get-by-uri",
        false,
        Output::Object,
        "A project by URI",
        [URI],
        get_by_uri
    ),
    command!(
        "projects",
        "at-location",
        false,
        Output::Object,
        "The project at a repository location",
        [LOCATION],
        at_location
    ),
    command!(
        "projects",
        "list",
        false,
        Output::Table(PROJECT_COLUMNS),
        "All projects you can see (walks the group tree)",
        [],
        list
    ),
    command!(
        "projects",
        "in-group",
        false,
        Output::Table(PROJECT_COLUMNS),
        "Projects in a group (with `deep`, also in subgroups)",
        [
            GROUP,
            Param::opt("deep", Kind::Boolean, "Include subgroups")
        ],
        in_group
    ),
    command!(
        "projects",
        "root-group",
        false,
        Output::Object,
        "The root project group",
        [],
        root_group
    ),
    command!(
        "projects",
        "group",
        false,
        Output::Object,
        "A project group by URI",
        [GROUP],
        group
    ),
    command!(
        "projects",
        "group-at",
        false,
        Output::Object,
        "The project group at a location",
        [LOCATION],
        group_at
    ),
    command!(
        "projects",
        "groups-in",
        false,
        Output::Table(GROUP_COLUMNS),
        "Subgroups of a group",
        [GROUP],
        groups_in
    ),
];

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let id = match a.opt_str("id")? {
        Some(id) => id,
        None => a.project()?,
    };
    out(p.projects().get(&id).await?)
}

async fn get_by_uri(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.projects().get_by_uri(&a.str("uri")?).await?)
}

async fn at_location(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.projects().at_location(&a.str("location")?).await?)
}

async fn list(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.projects().list().await?)
}

async fn in_group(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .projects()
        .in_group(&a.str("group")?, a.bool("deep")?)
        .await?)
}

async fn root_group(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.projects().root_group().await?)
}

async fn group(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.projects().group(&a.str("group")?).await?)
}

async fn group_at(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.projects().group_at(&a.str("location")?).await?)
}

async fn groups_in(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.projects().groups_in(&a.str("group")?).await?)
}
