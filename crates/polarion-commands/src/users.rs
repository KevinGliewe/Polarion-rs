//! `users.*`.

use polarion::{Polarion, UserUpdate};
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const ID: Param = Param::pos("id", Kind::String, "User id");

pub(crate) const COMMANDS: &[Command] = &[
    command!("users", "get", false, Output::Object, "A user", [ID], get),
    command!(
        "users",
        "get-by-uri",
        false,
        Output::Object,
        "A user by URI",
        [URI],
        get_by_uri
    ),
    command!(
        "users",
        "list",
        false,
        Output::Table(USER_COLUMNS),
        "All users",
        [],
        list
    ),
    command!(
        "users",
        "of-project",
        false,
        Output::Table(USER_COLUMNS),
        "Users with access to a project",
        [PROJECT],
        of_project
    ),
    command!(
        "users",
        "avatar-url",
        false,
        Output::Scalar,
        "The avatar URL of a user",
        [URI],
        avatar_url
    ),
    command!(
        "users",
        "create",
        true,
        Output::Object,
        "Create a user (needs POLARION_ALLOW_GLOBAL_WRITES)",
        [Param::pos("login", Kind::String, "Login name")],
        create
    ),
    command!(
        "users",
        "update",
        true,
        Output::Done,
        "Change a user (needs POLARION_ALLOW_GLOBAL_WRITES)",
        [
            ID,
            Param::opt("name", Kind::String, "Name"),
            Param::opt("email", Kind::String, "Email"),
            Param::opt("description", Kind::Text, "Description"),
            Param::opt(
                "disabled-notifications",
                Kind::Boolean,
                "Disable notifications"
            ),
        ],
        update
    ),
];

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.users().get(&a.str("id")?).await?)
}

async fn get_by_uri(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.users().get_by_uri(&a.str("uri")?).await?)
}

async fn list(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.users().list().await?)
}

async fn of_project(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.users().of_project(&a.project()?).await?)
}

async fn avatar_url(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.users().avatar_url(&a.str("uri")?).await?)
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.users().create(&a.str("login")?).await?)
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let update = UserUpdate {
        name: a.opt_str("name")?,
        email: a.opt_str("email")?,
        description: a.opt_text("description")?,
        disabled_notifications: a.opt_bool("disabled-notifications")?,
    };
    p.users().update(&a.str("id")?, &update).await?;
    Ok(Value::Null)
}
