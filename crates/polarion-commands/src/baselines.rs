//! `baselines.*`.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "baselines",
        "query",
        false,
        Output::Table(BASELINE_COLUMNS),
        "Search baselines with a Lucene query",
        [QUERY, PROJECT, SORT],
        query
    ),
    command!(
        "baselines",
        "create",
        true,
        Output::Object,
        "Create a baseline at a revision (HEAD if omitted)",
        [
            Param::pos("name", Kind::String, "Baseline name"),
            PROJECT,
            Param::opt("description", Kind::String, "Description"),
            Param::opt("revision", Kind::String, "Revision"),
        ],
        create
    ),
    command!(
        "baselines",
        "update",
        true,
        Output::Done,
        "Rename a baseline or change its description",
        [
            URI,
            Param::opt("name", Kind::String, "Name"),
            Param::opt("description", Kind::String, "Description")
        ],
        update
    ),
    command!(
        "baselines",
        "delete",
        true,
        Output::Done,
        "Delete a baseline",
        [URI],
        delete
    ),
];

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.baselines().query(&a.query()?).await?)
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let (description, revision) = (a.opt_str("description")?, a.opt_str("revision")?);
    out(p
        .baselines()
        .create(
            &a.project()?,
            &a.str("name")?,
            description.as_deref(),
            revision.as_deref(),
        )
        .await?)
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let (name, description) = (a.opt_str("name")?, a.opt_str("description")?);
    p.baselines()
        .update(&a.str("uri")?, name.as_deref(), description.as_deref())
        .await?;
    Ok(Value::Null)
}

async fn delete(p: &Polarion, a: &Args) -> Result<Value> {
    p.baselines().delete(&a.str("uri")?).await?;
    Ok(Value::Null)
}
