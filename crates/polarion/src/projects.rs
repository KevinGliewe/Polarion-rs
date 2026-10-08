//! Projects and users.

use std::collections::{BTreeSet, VecDeque};

use crate::client::{Polarion, not_found};
use crate::error::Result;
use crate::model::{Project, User};

/// Projects: [`Polarion::projects`].
#[derive(Debug, Clone, Copy)]
pub struct Projects<'a> {
    polarion: &'a Polarion,
}

impl<'a> Projects<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    pub async fn get(&self, id: &str) -> Result<Project> {
        let p = self.polarion.soap().project().get_project(id).await?;
        if p.unresolvable == Some(true) || (p.id.is_none() && p.uri.is_none()) {
            return Err(not_found("project", id));
        }
        Ok(p.into())
    }

    /// All projects the user can see, sorted by id. Walks the project group tree
    /// (two requests per group).
    pub async fn list(&self) -> Result<Vec<Project>> {
        let service = self.polarion.soap().project();
        let root = service.get_root_project_group().await?;
        let mut queue: VecDeque<String> = root.uri.into_iter().collect();
        let mut seen = BTreeSet::new();
        let mut projects: Vec<Project> = Vec::new();
        while let Some(group) = queue.pop_front() {
            if !seen.insert(group.clone()) {
                continue;
            }
            projects.extend(
                service
                    .get_contained_projects(&group)
                    .await?
                    .into_iter()
                    .filter(|p| p.unresolvable != Some(true))
                    .map(Project::from),
            );
            queue.extend(
                service
                    .get_contained_groups(&group)
                    .await?
                    .into_iter()
                    .filter_map(|g| g.uri),
            );
        }
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        projects.dedup_by(|a, b| a.id == b.id);
        Ok(projects)
    }
}

/// Users: [`Polarion::users`].
#[derive(Debug, Clone, Copy)]
pub struct Users<'a> {
    polarion: &'a Polarion,
}

impl<'a> Users<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    pub async fn get(&self, id: &str) -> Result<User> {
        let u = self.polarion.soap().project().get_user(id).await?;
        if u.unresolvable == Some(true) || (u.id.is_none() && u.uri.is_none()) {
            return Err(not_found("user", id));
        }
        Ok(u.into())
    }

    /// All users of the server.
    pub async fn list(&self) -> Result<Vec<User>> {
        let users = self.polarion.soap().project().get_users().await?;
        Ok(users.into_iter().map(User::from).collect())
    }

    /// The users with access to a project.
    pub async fn of_project(&self, project: &str) -> Result<Vec<User>> {
        let users = self
            .polarion
            .soap()
            .project()
            .get_project_users(project)
            .await?;
        Ok(users.into_iter().map(User::from).collect())
    }
}
