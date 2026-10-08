//! `security.*`: permissions, roles, licenses.

use polarion_commands_derive::cmd;

use crate::{Context, Done, Result, Run};

/// Whether you can read an object.
#[cmd(view = scalar)]
pub struct CanRead {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for CanRead {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().can_read(&self.uri).await?)
    }
}

/// Whether you can modify an object.
#[cmd(view = scalar)]
pub struct CanModify {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for CanModify {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().can_modify(&self.uri).await?)
    }
}

/// Whether you can delete an object.
#[cmd(view = scalar)]
pub struct CanDelete {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for CanDelete {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().can_delete(&self.uri).await?)
    }
}

/// Whether you can create objects of a prototype (e.g. WorkItem)
#[cmd(view = scalar)]
pub struct CanCreate {
    /// Prototype
    #[positional]
    pub prototype: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for CanCreate {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .can_create(&self.prototype, &cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Whether you can read a field.
#[cmd(view = scalar)]
pub struct CanReadField {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Field key
    pub key: String,
}

impl Run for CanReadField {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .can_read_field(&self.uri, &self.key)
            .await?)
    }
}

/// Whether you can modify a field.
#[cmd(view = scalar)]
pub struct CanModifyField {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Field key
    pub key: String,
}

impl Run for CanModifyField {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .can_modify_field(&self.uri, &self.key)
            .await?)
    }
}

/// Whether you can add to a collection field.
#[cmd(view = scalar)]
pub struct CanAddToField {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Field key
    pub key: String,
}

impl Run for CanAddToField {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .can_add_to_field(&self.uri, &self.key)
            .await?)
    }
}

/// Whether you can remove from a collection field.
#[cmd(view = scalar)]
pub struct CanRemoveFromField {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Field key
    pub key: String,
}

impl Run for CanRemoveFromField {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .can_remove_from_field(&self.uri, &self.key)
            .await?)
    }
}

/// Whether a user has a permission in a project.
#[cmd(view = scalar)]
pub struct HasPermission {
    /// User id
    pub user: String,
    /// Permission id
    pub permission: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for HasPermission {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .has_permission(
                &self.user,
                &self.permission,
                &cx.project(self.project.as_deref())?,
            )
            .await?)
    }
}

/// Whether you have a permission in a project.
#[cmd(view = scalar)]
pub struct CurrentUserHasPermission {
    /// Permission id
    pub permission: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for CurrentUserHasPermission {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .current_user_has_permission(&self.permission, &cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// A user's global roles and roles at a location.
#[cmd(view = lines)]
pub struct RolesForUser {
    /// User id
    pub user: String,
    /// Context location (global roles only if omitted)
    pub location: Option<String>,
}

impl Run for RolesForUser {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let location = self.location;
        Ok(cx
            .polarion
            .security()
            .roles_for_user(&self.user, location.as_deref())
            .await?)
    }
}

/// All global roles.
#[cmd(view = lines)]
pub struct GlobalRoles {}

impl Run for GlobalRoles {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().global_roles().await?)
    }
}

/// Roles defined in a context.
#[cmd(view = lines)]
pub struct ContextRoles {
    /// Context location (project or group)
    pub location: String,
}

impl Run for ContextRoles {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().context_roles(&self.location).await?)
    }
}

/// A user's roles in a context.
#[cmd(view = lines)]
pub struct ContextRolesForUser {
    /// User id
    pub user: String,
    /// Context location (project or group)
    pub location: String,
}

impl Run for ContextRolesForUser {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .context_roles_for_user(&self.user, &self.location)
            .await?)
    }
}

/// The repository location of an object.
#[cmd(view = scalar)]
pub struct LocationOf {
    /// Object URI
    #[positional]
    pub uri: String,
}

impl Run for LocationOf {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().location_of(&self.uri).await?)
    }
}

/// The user owning a personal access token.
#[cmd(view = scalar)]
pub struct UserFromToken {
    /// Token
    pub token: String,
}

impl Run for UserFromToken {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().user_from_token(&self.token).await?)
    }
}

/// The product license.
#[cmd(view = object)]
pub struct ProductLicense {}

impl Run for ProductLicense {
    type Output = polarion::soap::types::ProductLicense;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.security().product_license().await?)
    }
}

/// Whether an add-on is licensed.
#[cmd(view = scalar)]
pub struct HasAddonLicense {
    /// Add-on id
    #[positional]
    pub addon: String,
}

impl Run for HasAddonLicense {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .security()
            .has_addon_license(&self.addon)
            .await?)
    }
}

/// Store credentials in the account vault (needs POLARION_ALLOW_GLOBAL_WRITES)
#[cmd(write, view = done)]
pub struct UpdateUserAccountVault {
    /// Vault key
    pub key: String,
    /// User id
    pub user: String,
    /// Password
    pub password: String,
}

impl Run for UpdateUserAccountVault {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .security()
            .update_user_account_vault(&self.key, &self.user, &self.password)
            .await?;
        Ok(Done)
    }
}
