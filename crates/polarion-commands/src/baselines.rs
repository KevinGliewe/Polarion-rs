//! `baselines.*`.

use polarion_commands_derive::cmd;

use crate::{BASELINE_COLUMNS, Context, Done, QueryArgs, Result, Run};

/// Search baselines with a Lucene query.
#[cmd(view = table(BASELINE_COLUMNS))]
pub struct Query {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Query {
    type Output = Vec<polarion::Baseline>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .baselines()
            .query(&self.search.query(cx))
            .await?)
    }
}

/// Create a baseline at a revision (HEAD if omitted)
#[cmd(write, view = object)]
pub struct Create {
    /// Baseline name
    #[positional]
    pub name: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Revision
    pub revision: Option<String>,
}

impl Run for Create {
    type Output = polarion::Baseline;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (description, revision) = (self.description, self.revision);
        Ok(cx
            .polarion
            .baselines()
            .create(
                &cx.project(self.project.as_deref())?,
                &self.name,
                description.as_deref(),
                revision.as_deref(),
            )
            .await?)
    }
}

/// Rename a baseline or change its description.
#[cmd(write, view = done)]
pub struct Update {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Name
    pub name: Option<String>,
    /// Description
    pub description: Option<String>,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (name, description) = (self.name, self.description);
        cx.polarion
            .baselines()
            .update(&self.uri, name.as_deref(), description.as_deref())
            .await?;
        Ok(Done)
    }
}

/// Delete a baseline.
#[cmd(write, view = done)]
pub struct Delete {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for Delete {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion.baselines().delete(&self.uri).await?;
        Ok(Done)
    }
}
