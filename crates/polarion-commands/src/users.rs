//! `users.*`.

use polarion::UserUpdate;
use polarion_commands_derive::cmd;

use crate::{Context, Done, Result, RichText, Run, USER_COLUMNS};

/// A user.
#[cmd(view = object)]
pub struct Get {
    /// User id
    #[positional]
    pub id: String,
}

impl Run for Get {
    type Output = polarion::User;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.users().get(&self.id).await?)
    }
}

/// A user by URI.
#[cmd(view = object)]
pub struct GetByUri {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for GetByUri {
    type Output = polarion::User;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.users().get_by_uri(&self.uri).await?)
    }
}

/// All users.
#[cmd(view = table(USER_COLUMNS))]
pub struct List {}

impl Run for List {
    type Output = Vec<polarion::User>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.users().list().await?)
    }
}

/// Users with access to a project.
#[cmd(view = table(USER_COLUMNS))]
pub struct OfProject {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for OfProject {
    type Output = Vec<polarion::User>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .users()
            .of_project(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// The avatar URL of a user.
#[cmd(view = scalar)]
pub struct AvatarUrl {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for AvatarUrl {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.users().avatar_url(&self.uri).await?)
    }
}

/// Create a user (needs POLARION_ALLOW_GLOBAL_WRITES)
#[cmd(write, view = object)]
pub struct Create {
    /// Login name
    #[positional]
    pub login: String,
}

impl Run for Create {
    type Output = polarion::User;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.users().create(&self.login).await?)
    }
}

/// Change a user (needs POLARION_ALLOW_GLOBAL_WRITES)
#[cmd(write, view = done)]
pub struct Update {
    /// User id
    #[positional]
    pub id: String,
    /// Name
    pub name: Option<String>,
    /// Email
    pub email: Option<String>,
    /// Description
    pub description: Option<RichText>,
    /// Disable notifications
    pub disabled_notifications: Option<bool>,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let update = UserUpdate {
            name: self.name,
            email: self.email,
            description: self.description.map(polarion::Text::from),
            disabled_notifications: self.disabled_notifications,
        };
        cx.polarion.users().update(&self.id, &update).await?;
        Ok(Done)
    }
}
