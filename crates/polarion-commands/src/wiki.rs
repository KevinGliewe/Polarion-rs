//! `wiki.*`: wiki pages.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const SPACE: Param = Param::req("space", Kind::String, "Wiki space id");

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "wiki",
        "spaces",
        false,
        Output::Lines,
        "Wiki spaces of a project",
        [PROJECT],
        spaces
    ),
    command!(
        "wiki",
        "pages",
        false,
        Output::Table(PAGE_COLUMNS),
        "Pages of a space (all fields, or only `fields`)",
        [PROJECT, SPACE, FIELDS],
        pages
    ),
    command!(
        "wiki",
        "page-uris",
        false,
        Output::Lines,
        "URIs of the pages of a space",
        [PROJECT, SPACE],
        page_uris
    ),
    command!(
        "wiki",
        "get",
        false,
        Output::Object,
        "One page by URI (all fields, or only `fields`)",
        [URI, FIELDS],
        get
    ),
    command!(
        "wiki",
        "query",
        false,
        Output::Table(PAGE_COLUMNS),
        "Search pages with a Lucene query",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS, BASELINE],
        query
    ),
    command!(
        "wiki",
        "query-uris",
        false,
        Output::Lines,
        "Search pages, returning URIs",
        [QUERY, PROJECT, SORT, LIMIT, ALL, BASELINE],
        query_uris
    ),
    command!(
        "wiki",
        "query-sql",
        false,
        Output::Table(PAGE_COLUMNS),
        "Select pages with SQL",
        [SQL, FIELDS, BASELINE],
        query_sql
    ),
    command!(
        "wiki",
        "query-uris-sql",
        false,
        Output::Lines,
        "Select page URIs with SQL",
        [SQL, BASELINE],
        query_uris_sql
    ),
];

async fn spaces(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.wiki().spaces(&a.project()?).await?)
}

async fn pages(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p
        .wiki()
        .pages(&a.project()?, &a.str("space")?, fields.as_ref())
        .await?)
}

async fn page_uris(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.wiki().page_uris(&a.project()?, &a.str("space")?).await?)
}

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p.wiki().get(&a.str("uri")?, fields.as_ref()).await?)
}

async fn query(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.wiki().query(&a.query()?).await?)
}

async fn query_uris(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.wiki().query_uris(&a.query()?).await?)
}

async fn query_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let (fields, baseline) = (a.opt_fields()?, a.opt_str("baseline")?);
    out(p
        .wiki()
        .query_sql(&a.str("sql")?, fields.as_ref(), baseline.as_deref())
        .await?)
}

async fn query_uris_sql(p: &Polarion, a: &Args) -> Result<Value> {
    let baseline = a.opt_str("baseline")?;
    out(p
        .wiki()
        .query_uris_sql(&a.str("sql")?, baseline.as_deref())
        .await?)
}
