//! `security.*`: permissions, roles, licenses.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const KEY: Param = Param::req("key", Kind::String, "Field key");
const USER: Param = Param::req("user", Kind::String, "User id");
const PERMISSION: Param = Param::req("permission", Kind::String, "Permission id");
const LOCATION: Param = Param::req(
    "location",
    Kind::String,
    "Context location (project or group)",
);

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "security",
        "can-read",
        false,
        Output::Scalar,
        "Whether you can read an object",
        [URI],
        can_read
    ),
    command!(
        "security",
        "can-modify",
        false,
        Output::Scalar,
        "Whether you can modify an object",
        [URI],
        can_modify
    ),
    command!(
        "security",
        "can-delete",
        false,
        Output::Scalar,
        "Whether you can delete an object",
        [URI],
        can_delete
    ),
    command!(
        "security",
        "can-create",
        false,
        Output::Scalar,
        "Whether you can create objects of a prototype (e.g. WorkItem)",
        [Param::pos("prototype", Kind::String, "Prototype"), PROJECT],
        can_create
    ),
    command!(
        "security",
        "can-read-field",
        false,
        Output::Scalar,
        "Whether you can read a field",
        [URI, KEY],
        can_read_field
    ),
    command!(
        "security",
        "can-modify-field",
        false,
        Output::Scalar,
        "Whether you can modify a field",
        [URI, KEY],
        can_modify_field
    ),
    command!(
        "security",
        "can-add-to-field",
        false,
        Output::Scalar,
        "Whether you can add to a collection field",
        [URI, KEY],
        can_add_to_field
    ),
    command!(
        "security",
        "can-remove-from-field",
        false,
        Output::Scalar,
        "Whether you can remove from a collection field",
        [URI, KEY],
        can_remove_from_field
    ),
    command!(
        "security",
        "has-permission",
        false,
        Output::Scalar,
        "Whether a user has a permission in a project",
        [USER, PERMISSION, PROJECT],
        has_permission
    ),
    command!(
        "security",
        "current-user-has-permission",
        false,
        Output::Scalar,
        "Whether you have a permission in a project",
        [PERMISSION, PROJECT],
        current_user_has_permission
    ),
    command!(
        "security",
        "roles-for-user",
        false,
        Output::Lines,
        "A user's global roles and roles at a location",
        [
            USER,
            Param::opt(
                "location",
                Kind::String,
                "Context location (global roles only if omitted)"
            )
        ],
        roles_for_user
    ),
    command!(
        "security",
        "global-roles",
        false,
        Output::Lines,
        "All global roles",
        [],
        global_roles
    ),
    command!(
        "security",
        "context-roles",
        false,
        Output::Lines,
        "Roles defined in a context",
        [LOCATION],
        context_roles
    ),
    command!(
        "security",
        "context-roles-for-user",
        false,
        Output::Lines,
        "A user's roles in a context",
        [USER, LOCATION],
        context_roles_for_user
    ),
    command!(
        "security",
        "location-of",
        false,
        Output::Scalar,
        "The repository location of an object",
        [URI],
        location_of
    ),
    command!(
        "security",
        "user-from-token",
        false,
        Output::Scalar,
        "The user owning a personal access token",
        [Param::req("token", Kind::String, "Token")],
        user_from_token
    ),
    command!(
        "security",
        "product-license",
        false,
        Output::Object,
        "The product license",
        [],
        product_license
    ),
    command!(
        "security",
        "has-addon-license",
        false,
        Output::Scalar,
        "Whether an add-on is licensed",
        [Param::pos("addon", Kind::String, "Add-on id")],
        has_addon_license
    ),
    command!(
        "security",
        "update-user-account-vault",
        true,
        Output::Done,
        "Store credentials in the account vault (needs POLARION_ALLOW_GLOBAL_WRITES)",
        [
            Param::req("key", Kind::String, "Vault key"),
            USER,
            Param::req("password", Kind::String, "Password")
        ],
        update_user_account_vault
    ),
];

async fn can_read(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().can_read(&a.str("uri")?).await?)
}

async fn can_modify(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().can_modify(&a.str("uri")?).await?)
}

async fn can_delete(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().can_delete(&a.str("uri")?).await?)
}

async fn can_create(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .can_create(&a.str("prototype")?, &a.project()?)
        .await?)
}

async fn can_read_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .can_read_field(&a.str("uri")?, &a.str("key")?)
        .await?)
}

async fn can_modify_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .can_modify_field(&a.str("uri")?, &a.str("key")?)
        .await?)
}

async fn can_add_to_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .can_add_to_field(&a.str("uri")?, &a.str("key")?)
        .await?)
}

async fn can_remove_from_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .can_remove_from_field(&a.str("uri")?, &a.str("key")?)
        .await?)
}

async fn has_permission(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .has_permission(&a.str("user")?, &a.str("permission")?, &a.project()?)
        .await?)
}

async fn current_user_has_permission(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .current_user_has_permission(&a.str("permission")?, &a.project()?)
        .await?)
}

async fn roles_for_user(p: &Polarion, a: &Args) -> Result<Value> {
    let location = a.opt_str("location")?;
    out(p
        .security()
        .roles_for_user(&a.str("user")?, location.as_deref())
        .await?)
}

async fn global_roles(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.security().global_roles().await?)
}

async fn context_roles(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().context_roles(&a.str("location")?).await?)
}

async fn context_roles_for_user(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .security()
        .context_roles_for_user(&a.str("user")?, &a.str("location")?)
        .await?)
}

async fn location_of(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().location_of(&a.str("uri")?).await?)
}

async fn user_from_token(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().user_from_token(&a.str("token")?).await?)
}

async fn product_license(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.security().product_license().await?)
}

async fn has_addon_license(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.security().has_addon_license(&a.str("addon")?).await?)
}

async fn update_user_account_vault(p: &Polarion, a: &Args) -> Result<Value> {
    p.security()
        .update_user_account_vault(&a.str("key")?, &a.str("user")?, &a.str("password")?)
        .await?;
    Ok(Value::Null)
}
