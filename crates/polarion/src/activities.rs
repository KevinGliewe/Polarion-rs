//! Activity streams (changes, builds, ... as shown in Polarion's activity views).

use polarion_soap::services::TrackerService;
use polarion_soap::types::{Activity, ActivitySource};

use crate::client::Polarion;
use crate::convert::soap_text;
use crate::error::Result;
use crate::model::Text;

/// Where to list activities from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityScope<'s> {
    /// The whole default repository.
    All,
    /// One project, by id.
    Project(&'s str),
    /// A project group, by URI.
    Group(&'s str),
}

/// Activities: [`Polarion::activities`]. Results are the SOAP [`Activity`] type.
#[derive(Debug, Clone, Copy)]
pub struct Activities<'a> {
    polarion: &'a Polarion,
}

impl<'a> Activities<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn tracker(&self) -> TrackerService<'a> {
        self.polarion.soap().tracker()
    }

    /// The latest `count` activities in a scope, optionally filtered by a Lucene query.
    pub async fn list(
        &self,
        scope: ActivityScope<'_>,
        query: Option<&str>,
        count: u32,
    ) -> Result<Vec<Activity>> {
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        let t = self.tracker();
        Ok(match scope {
            ActivityScope::All => t.list_all_activities(query, count).await?,
            ActivityScope::Project(p) => t.list_project_activities(p, query, count).await?,
            ActivityScope::Group(g) => t.list_group_activities(g, query, count).await?,
        })
    }

    /// Global ids of the latest `count` activities in a scope.
    pub async fn list_ids(
        &self,
        scope: ActivityScope<'_>,
        query: Option<&str>,
        count: u32,
    ) -> Result<Vec<String>> {
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        let t = self.tracker();
        Ok(match scope {
            ActivityScope::All => t.list_all_activities_global_ids(query, count).await?,
            ActivityScope::Project(p) => {
                t.list_project_activities_global_ids(p, query, count)
                    .await?
            }
            ActivityScope::Group(g) => t.list_group_activities_global_ids(g, query, count).await?,
        })
    }

    pub async fn get(&self, global_id: &str) -> Result<Activity> {
        Ok(self.tracker().get_activity_by_global_id(global_id).await?)
    }

    /// The kinds of activity sources (work items, builds, ...).
    pub async fn sources(&self) -> Result<Vec<ActivitySource>> {
        Ok(self.tracker().list_activity_sources().await?)
    }

    /// Whether the activity accepts comments at all.
    pub async fn can_comment(&self, global_id: &str) -> Result<bool> {
        Ok(self.tracker().can_comment_activity(global_id).await?)
    }

    /// Whether the current user may comment on the activity.
    pub async fn can_current_user_comment(&self, global_id: &str) -> Result<bool> {
        Ok(self
            .tracker()
            .can_current_user_add_comment_to_activity(global_id)
            .await?)
    }

    /// Comment on an activity. Activities are not tied to a project here, so this needs
    /// [`crate::PolarionBuilder::allow_global_writes`].
    pub async fn add_comment(&self, global_id: &str, text: &Text) -> Result<()> {
        self.polarion.check_global_write("addCommentToActivity")?;
        Ok(self
            .tracker()
            .add_comment_to_activity(global_id, &soap_text(text))
            .await?)
    }
}
