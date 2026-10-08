//! Permissions, roles and licensing.

use polarion_soap::services::SecurityService;
use polarion_soap::types::ProductLicense;

use crate::client::Polarion;
use crate::error::Result;

/// Security: [`Polarion::security`]. Permission checks are for the logged-in user unless a
/// user is named.
#[derive(Debug, Clone, Copy)]
pub struct Security<'a> {
    polarion: &'a Polarion,
}

impl<'a> Security<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn service(&self) -> SecurityService<'a> {
        self.polarion.soap().security()
    }

    pub async fn can_read(&self, uri: &str) -> Result<bool> {
        Ok(self.service().can_read_instance(uri).await?)
    }

    pub async fn can_modify(&self, uri: &str) -> Result<bool> {
        Ok(self.service().can_modify_instance(uri).await?)
    }

    pub async fn can_delete(&self, uri: &str) -> Result<bool> {
        Ok(self.service().can_delete_instance(uri).await?)
    }

    /// Whether objects of a prototype (e.g. `WorkItem`) can be created in a project.
    pub async fn can_create(&self, prototype: &str, project: &str) -> Result<bool> {
        Ok(self
            .service()
            .can_create_instances(prototype, project)
            .await?)
    }

    pub async fn can_read_field(&self, uri: &str, key: &str) -> Result<bool> {
        Ok(self.service().can_read_key(uri, key).await?)
    }

    pub async fn can_modify_field(&self, uri: &str, key: &str) -> Result<bool> {
        Ok(self.service().can_modify_key(uri, key).await?)
    }

    /// Whether elements can be added to a collection field (e.g. `linkedWorkItems`).
    pub async fn can_add_to_field(&self, uri: &str, key: &str) -> Result<bool> {
        Ok(self.service().can_add_element_to_key(uri, key).await?)
    }

    pub async fn can_remove_from_field(&self, uri: &str, key: &str) -> Result<bool> {
        Ok(self.service().can_remove_element_from_key(uri, key).await?)
    }

    /// Whether a user has a permission (e.g. `com.polarion.persistence.object.WorkItem.modify`)
    /// in a project.
    pub async fn has_permission(
        &self,
        user: &str,
        permission: &str,
        project: &str,
    ) -> Result<bool> {
        Ok(self
            .service()
            .has_permission(user, permission, project)
            .await?)
    }

    pub async fn current_user_has_permission(
        &self,
        permission: &str,
        project: &str,
    ) -> Result<bool> {
        Ok(self
            .service()
            .has_current_user_permission(permission, project)
            .await?)
    }

    /// A user's global roles and the roles at a location (`None`: global roles only).
    pub async fn roles_for_user(&self, user: &str, location: Option<&str>) -> Result<Vec<String>> {
        Ok(self.service().get_roles_for_user(user, location).await?)
    }

    /// All global roles.
    pub async fn global_roles(&self) -> Result<Vec<String>> {
        Ok(self.service().get_global_roles().await?)
    }

    /// The roles defined in a context (project or project group location).
    pub async fn context_roles(&self, location: &str) -> Result<Vec<String>> {
        Ok(self.service().get_context_roles(location).await?)
    }

    /// A user's roles in a context.
    pub async fn context_roles_for_user(&self, user: &str, location: &str) -> Result<Vec<String>> {
        Ok(self
            .service()
            .get_context_roles_for_user(user, location)
            .await?)
    }

    /// The repository location of an object (for the role methods).
    pub async fn location_of(&self, uri: &str) -> Result<String> {
        Ok(self.service().get_location_for_uri(uri).await?)
    }

    /// The user owning a personal access token.
    pub async fn user_from_token(&self, token: &str) -> Result<String> {
        Ok(self.service().get_user_from_token(token).await?)
    }

    /// The product license (not available with SALT cloud licensing).
    pub async fn product_license(&self) -> Result<ProductLicense> {
        Ok(self.service().get_product_license().await?)
    }

    pub async fn has_addon_license(&self, addon: &str) -> Result<bool> {
        Ok(self.service().has_addon_license(addon).await?)
    }

    /// Store credentials in the user account vault (administrators only). Needs
    /// [`crate::PolarionBuilder::allow_global_writes`].
    pub async fn update_user_account_vault(
        &self,
        key: &str,
        user: &str,
        password: &str,
    ) -> Result<()> {
        self.polarion.check_global_write("updateUserAccountVault")?;
        Ok(self
            .service()
            .update_user_account_vault(key, user, password)
            .await?)
    }
}
