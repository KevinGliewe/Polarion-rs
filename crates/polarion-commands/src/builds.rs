//! `builds.*`.

use polarion_commands_derive::cmd;

use crate::{BUILD_COLUMNS, Context, Result, Run};

/// All builds on the server.
#[cmd(view = table(BUILD_COLUMNS))]
pub struct All {}

impl Run for All {
    type Output = Vec<polarion::soap::types::Build>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.builds().all().await?)
    }
}

/// Builds of a project.
#[cmd(view = table(BUILD_COLUMNS))]
pub struct OfProject {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for OfProject {
    type Output = Vec<polarion::soap::types::Build>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .builds()
            .of_project(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Builds of a project matching a Lucene query.
#[cmd(view = table(BUILD_COLUMNS))]
pub struct Query {
    /// Lucene query
    #[positional]
    pub query: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Sort order, e.g. `id`, `~updated` (descending)
    pub sort: Option<String>,
}

impl Run for Query {
    type Output = Vec<polarion::soap::types::Build>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let sort = self.sort.unwrap_or_default();
        Ok(cx
            .polarion
            .builds()
            .query(&cx.project(self.project.as_deref())?, &self.query, &sort)
            .await?)
    }
}
