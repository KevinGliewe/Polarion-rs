//! `revisions.*`: source repository revisions.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "revisions",
        "get",
        false,
        Output::Object,
        "A revision of a repository",
        [
            Param::pos("revision", Kind::String, "Revision id"),
            Param::opt(
                "repository",
                Kind::String,
                "Repository (default: `default`)"
            )
        ],
        get
    ),
    command!(
        "revisions",
        "get-by-uri",
        false,
        Output::Object,
        "A revision by URI",
        [URI],
        get_by_uri
    ),
    command!(
        "revisions",
        "query",
        false,
        Output::Table(REVISION_COLUMNS),
        "Search revisions with a Lucene query",
        [QUERY, SORT, FIELDS],
        query
    ),
    command!(
        "revisions",
        "query-uris",
        false,
        Output::Lines,
        "Search revisions, returning URIs",
        [
            QUERY,
            SORT,
            Param::opt(
                "include-internal",
                Kind::Boolean,
                "Include Polarion's own commits"
            )
        ],
        query_uris
    ),
    command!(
        "revisions",
        "work-items",
        false,
        Output::Table(ITEM_COLUMNS),
        "Work items linked to a revision",
        [URI, FIELDS, CUSTOM],
        work_items
    ),
];

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let repository = a.opt_str("repository")?.unwrap_or_else(|| "default".into());
    out(p.revisions().get(&repository, &a.str("revision")?).await?)
}

async fn get_by_uri(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.revisions().get_by_uri(&a.str("uri")?).await?)
}

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.revisions().query(&a.query()?).await?)
}

async fn query_uris(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .revisions()
        .query_uris(&a.query()?, a.bool("include-internal")?)
        .await?)
}

async fn work_items(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p
        .revisions()
        .work_items(&a.str("uri")?, fields.as_ref())
        .await?)
}
