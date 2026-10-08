//! Builds (Polarion's build management).

use polarion_soap::types::{self as soap, Build};

use crate::client::{Polarion, not_found};
use crate::error::Result;

/// Builds: [`Polarion::builds`]. Results are the SOAP [`Build`] type.
#[derive(Debug, Clone, Copy)]
pub struct Builds<'a> {
    polarion: &'a Polarion,
}

impl<'a> Builds<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    /// All builds on the server.
    pub async fn all(&self) -> Result<Vec<Build>> {
        Ok(self.polarion.soap().builds().get_all_builds().await?)
    }

    /// The builds of a project.
    pub async fn of_project(&self, project: &str) -> Result<Vec<Build>> {
        let project = self.project(project).await?;
        Ok(self.polarion.soap().builds().get_builds(&project).await?)
    }

    /// Builds of a project matching a Lucene query.
    pub async fn query(&self, project: &str, query: &str, sort: &str) -> Result<Vec<Build>> {
        let project = self.project(project).await?;
        Ok(self
            .polarion
            .soap()
            .builds()
            .query_builds(&project, query, sort)
            .await?)
    }

    /// The project reference the Builder service expects.
    async fn project(&self, id: &str) -> Result<soap::Project> {
        let p = self.polarion.soap().project().get_project(id).await?;
        if p.unresolvable == Some(true) || p.uri.is_none() {
            return Err(not_found("project", id));
        }
        Ok(soap::Project {
            id: p.id,
            uri: p.uri,
            ..soap::Project::default()
        })
    }
}
