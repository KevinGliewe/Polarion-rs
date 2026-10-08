//! `wiki.*`: wiki pages.

use polarion_commands_derive::cmd;

use crate::{Context, FieldSelection, PAGE_COLUMNS, QueryArgs, Result, Run};

/// Wiki spaces of a project.
#[cmd(view = lines)]
pub struct Spaces {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Spaces {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .wiki()
            .spaces(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Pages of a space (all fields, or only `fields`)
#[cmd(view = table(PAGE_COLUMNS))]
pub struct Pages {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Wiki space id
    pub space: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
}

impl Run for Pages {
    type Output = Vec<polarion::WikiPage>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &[]);
        Ok(cx
            .polarion
            .wiki()
            .pages(
                &cx.project(self.project.as_deref())?,
                &self.space,
                fields.as_ref(),
            )
            .await?)
    }
}

/// URIs of the pages of a space.
#[cmd(view = lines)]
pub struct PageUris {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Wiki space id
    pub space: String,
}

impl Run for PageUris {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .wiki()
            .page_uris(&cx.project(self.project.as_deref())?, &self.space)
            .await?)
    }
}

/// One page by URI (all fields, or only `fields`)
#[cmd(view = object)]
pub struct Get {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
}

impl Run for Get {
    type Output = polarion::WikiPage;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &[]);
        Ok(cx.polarion.wiki().get(&self.uri, fields.as_ref()).await?)
    }
}

/// Search pages with a Lucene query.
#[cmd(view = table(PAGE_COLUMNS))]
pub struct Query {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Query {
    type Output = Vec<polarion::WikiPage>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.wiki().query(&self.search.query(cx)).await?)
    }
}

/// Search pages, returning URIs.
#[cmd(view = lines)]
pub struct QueryUris {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for QueryUris {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .wiki()
            .query_uris(&self.search.query(cx))
            .await?)
    }
}

/// Select pages with SQL.
#[cmd(view = table(PAGE_COLUMNS))]
pub struct QuerySql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QuerySql {
    type Output = Vec<polarion::WikiPage>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (fields, baseline) = (
            FieldSelection::fields(self.fields.as_ref(), &[]),
            self.baseline,
        );
        Ok(cx
            .polarion
            .wiki()
            .query_sql(&self.sql, fields.as_ref(), baseline.as_deref())
            .await?)
    }
}

/// Select page URIs with SQL.
#[cmd(view = lines)]
pub struct QueryUrisSql {
    /// SQL query (SELECT ... FROM ...)
    #[positional]
    pub sql: String,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
}

impl Run for QueryUrisSql {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let baseline = self.baseline;
        Ok(cx
            .polarion
            .wiki()
            .query_uris_sql(&self.sql, baseline.as_deref())
            .await?)
    }
}
