//! `revisions.*`: source repository revisions.

use polarion_commands_derive::cmd;

use crate::{Context, FieldSelection, ITEM_COLUMNS, QueryArgs, REVISION_COLUMNS, Result, Run};

/// A revision of a repository.
#[cmd(view = object)]
pub struct Get {
    /// Revision id
    #[positional]
    pub revision: String,
    /// Repository (default: `default`)
    pub repository: Option<String>,
}

impl Run for Get {
    type Output = polarion::Revision;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let repository = self.repository.unwrap_or_else(|| "default".into());
        Ok(cx
            .polarion
            .revisions()
            .get(&repository, &self.revision)
            .await?)
    }
}

/// A revision by URI.
#[cmd(view = object)]
pub struct GetByUri {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for GetByUri {
    type Output = polarion::Revision;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.revisions().get_by_uri(&self.uri).await?)
    }
}

/// Search revisions with a Lucene query.
#[cmd(view = table(REVISION_COLUMNS))]
pub struct Query {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Query {
    type Output = Vec<polarion::Revision>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .revisions()
            .query(&self.search.query(cx))
            .await?)
    }
}

/// Search revisions, returning URIs.
#[cmd(view = lines)]
pub struct QueryUris {
    /// Include Polarion's own commits
    pub include_internal: bool,
    #[flatten]
    pub search: QueryArgs,
}

impl Run for QueryUris {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .revisions()
            .query_uris(&self.search.query(cx), self.include_internal)
            .await?)
    }
}

/// Work items linked to a revision.
#[cmd(view = table(ITEM_COLUMNS))]
pub struct WorkItems {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
}

impl Run for WorkItems {
    type Output = Vec<polarion::WorkItem>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &self.custom);
        Ok(cx
            .polarion
            .revisions()
            .work_items(&self.uri, fields.as_ref())
            .await?)
    }
}
