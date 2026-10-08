//! Projects and users.

use std::collections::{BTreeSet, VecDeque};

use crate::client::{Polarion, not_found};
use crate::convert::soap_text;
use crate::error::Result;
use crate::model::{Project, ProjectGroup, User};
use crate::write::UserUpdate;

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

    pub async fn get_by_uri(&self, uri: &str) -> Result<Project> {
        let p = self
            .polarion
            .soap()
            .project()
            .get_project_by_uri(uri)
            .await?;
        if p.unresolvable == Some(true) || (p.id.is_none() && p.uri.is_none()) {
            return Err(not_found("project", uri));
        }
        Ok(p.into())
    }

    /// The project at a repository location.
    pub async fn at_location(&self, location: &str) -> Result<Project> {
        let p = self
            .polarion
            .soap()
            .project()
            .get_project_at_location(location)
            .await?;
        if p.unresolvable == Some(true) || (p.id.is_none() && p.uri.is_none()) {
            return Err(not_found("project", location));
        }
        Ok(p.into())
    }

    /// The projects in a project group; with `deep`, also those in its subgroups.
    pub async fn in_group(&self, group_uri: &str, deep: bool) -> Result<Vec<Project>> {
        let service = self.polarion.soap().project();
        let list = if deep {
            service.get_deep_contained_projects(group_uri).await?
        } else {
            service.get_contained_projects(group_uri).await?
        };
        Ok(list.into_iter().map(Project::from).collect())
    }

    /// The root of the project group tree.
    pub async fn root_group(&self) -> Result<ProjectGroup> {
        Ok(self
            .polarion
            .soap()
            .project()
            .get_root_project_group()
            .await?
            .into())
    }

    pub async fn group(&self, uri: &str) -> Result<ProjectGroup> {
        Ok(self
            .polarion
            .soap()
            .project()
            .get_project_group(uri)
            .await?
            .into())
    }

    pub async fn group_at(&self, location: &str) -> Result<ProjectGroup> {
        Ok(self
            .polarion
            .soap()
            .project()
            .get_project_group_at_location(location)
            .await?
            .into())
    }

    /// The direct subgroups of a project group.
    pub async fn groups_in(&self, group_uri: &str) -> Result<Vec<ProjectGroup>> {
        let groups = self
            .polarion
            .soap()
            .project()
            .get_contained_groups(group_uri)
            .await?;
        Ok(groups.into_iter().map(ProjectGroup::from).collect())
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

    pub async fn get_by_uri(&self, uri: &str) -> Result<User> {
        let u = self.polarion.soap().project().get_user_by_uri(uri).await?;
        if u.unresolvable == Some(true) || (u.id.is_none() && u.uri.is_none()) {
            return Err(not_found("user", uri));
        }
        Ok(u.into())
    }

    /// The URL of a user's avatar (absolute, or relative to the portal), by user URI.
    pub async fn avatar_url(&self, user_uri: &str) -> Result<String> {
        Ok(self
            .polarion
            .soap()
            .project()
            .get_user_avatar_url(user_uri)
            .await?)
    }

    /// Create a user with a login name. Needs [`crate::PolarionBuilder::allow_global_writes`].
    pub async fn create(&self, login: &str) -> Result<User> {
        self.polarion.check_global_write("createUser")?;
        Ok(self
            .polarion
            .soap()
            .project()
            .create_user(login)
            .await?
            .into())
    }

    /// Change a user's name, email, description or notification setting. Needs
    /// [`crate::PolarionBuilder::allow_global_writes`].
    pub async fn update(&self, id: &str, update: &UserUpdate) -> Result<()> {
        self.polarion.check_global_write("updateUser")?;
        let service = self.polarion.soap().project();
        let mut user = service.get_user(id).await?;
        if user.unresolvable == Some(true) || (user.id.is_none() && user.uri.is_none()) {
            return Err(not_found("user", id));
        }
        if let Some(name) = &update.name {
            user.name = Some(name.clone());
        }
        if let Some(email) = &update.email {
            user.email = Some(email.clone());
        }
        if let Some(description) = &update.description {
            user.description = Some(soap_text(description));
        }
        if let Some(disabled) = update.disabled_notifications {
            user.disabled_notifications = Some(disabled);
        }
        Ok(service.update_user(&user).await?)
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
