//! `builds.*`.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "builds",
        "all",
        false,
        Output::Table(BUILD_COLUMNS),
        "All builds on the server",
        [],
        all
    ),
    command!(
        "builds",
        "of-project",
        false,
        Output::Table(BUILD_COLUMNS),
        "Builds of a project",
        [PROJECT],
        of_project
    ),
    command!(
        "builds",
        "query",
        false,
        Output::Table(BUILD_COLUMNS),
        "Builds of a project matching a Lucene query",
        [
            Param::pos("query", Kind::String, "Lucene query"),
            PROJECT,
            SORT
        ],
        query
    ),
];

async fn all(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.builds().all().await?)
}

async fn of_project(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.builds().of_project(&a.project()?).await?)
}

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    let sort = a.opt_str("sort")?.unwrap_or_default();
    out(p
        .builds()
        .query(&a.project()?, &a.str("query")?, &sort)
        .await?)
}
