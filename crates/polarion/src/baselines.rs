//! Baselines: named repository revisions.

use polarion_soap::types as soap;

use crate::client::Polarion;
use crate::error::Result;
use crate::model::Baseline;
use crate::query::Query;

/// Baselines: [`Polarion::baselines`].
#[derive(Debug, Clone, Copy)]
pub struct Baselines<'a> {
    polarion: &'a Polarion,
}

impl<'a> Baselines<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    /// Baselines matching a Lucene query (limit, fields and baseline are ignored).
    pub async fn query(&self, query: &Query) -> Result<Vec<Baseline>> {
        let list = self
            .polarion
            .soap()
            .tracker()
            .query_baselines(&query.lucene(), query.sort_key())
            .await?;
        Ok(list.into_iter().map(Baseline::from).collect())
    }

    /// Create a baseline of a project at a revision (`None`: HEAD).
    pub async fn create(
        &self,
        project: &str,
        name: &str,
        description: Option<&str>,
        revision: Option<&str>,
    ) -> Result<Baseline> {
        self.polarion.check_unbounded_write("createBaseline")?;
        self.polarion.check_write("createBaseline", Some(project))?;
        let b = self
            .polarion
            .soap()
            .tracker()
            .create_baseline(project, name, description, revision)
            .await?;
        Ok(b.into())
    }

    /// Rename a baseline or change its description (`None` leaves a field unchanged).
    pub async fn update(
        &self,
        uri: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        self.polarion.check_write_target("updateBaseline", uri)?;
        let baseline = soap::Baseline {
            uri: Some(uri.to_owned()),
            name: name.map(str::to_owned),
            description: description.map(str::to_owned),
            ..soap::Baseline::default()
        };
        Ok(self
            .polarion
            .soap()
            .tracker()
            .update_baseline(&baseline)
            .await?)
    }

    pub async fn delete(&self, uri: &str) -> Result<()> {
        self.polarion.check_unbounded_write("deleteBaseline")?;
        self.polarion.check_write_uri("deleteBaseline", uri)?;
        Ok(self.polarion.soap().tracker().delete_baseline(uri).await?)
    }
}
