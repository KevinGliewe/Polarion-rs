//! Every operation of the [`polarion`] library as a named command that takes and returns
//! JSON. The command-line tool and the MCP server are both generated from this catalog.
//!
//! ```no_run
//! use polarion_commands::{Args, Config, find};
//! use serde_json::json;
//!
//! # async fn run() -> polarion_commands::Result<()> {
//! let config = Config::from_env();
//! let polarion = config.connect()?;
//! let command = find("items.get").unwrap();
//! let args = Args::from_value(json!({"item": "MyProject/MP-42"}), config.project.clone())?;
//! let item = (command.run)(&polarion, &args).await?;
//! println!("{item}");
//! # Ok(()) }
//! ```

mod args;
mod catalog;
mod config;
mod error;
mod util;

mod activities;
mod baselines;
mod builds;
mod documents;
mod folders;
mod items;
mod plans;
mod projects;
mod revisions;
mod schema;
mod security;
mod session;
mod tests_mgmt;
mod users;
mod wiki;

use std::sync::OnceLock;

pub use args::{Args, infer};
pub use catalog::*;
pub use config::{Config, split_list};
pub use error::{CommandError, Result};

/// All commands, grouped by area in a fixed order.
pub fn commands() -> &'static [Command] {
    static ALL: OnceLock<Vec<Command>> = OnceLock::new();
    ALL.get_or_init(|| {
        let areas: [&[Command]; 15] = [
            items::COMMANDS,
            documents::COMMANDS,
            schema::COMMANDS,
            tests_mgmt::COMMANDS,
            plans::COMMANDS,
            wiki::COMMANDS,
            baselines::COMMANDS,
            revisions::COMMANDS,
            activities::COMMANDS,
            folders::COMMANDS,
            projects::COMMANDS,
            users::COMMANDS,
            security::COMMANDS,
            builds::COMMANDS,
            session::COMMANDS,
        ];
        areas.into_iter().flatten().copied().collect()
    })
}

/// The areas, in catalog order, with a one-line description.
pub const AREAS: &[(&str, &str)] = &[
    (
        "items",
        "Work items: read, query, history, workflow, links, comments, create, update",
    ),
    (
        "documents",
        "Documents (live docs): list, read, query, work items, create, update, comments",
    ),
    (
        "schema",
        "Project configuration: custom fields, enumerations, workflow, languages",
    ),
    (
        "tests",
        "Test management: test runs, records, steps, attachments",
    ),
    (
        "plans",
        "Planning: plans and templates, statistics, planned items",
    ),
    ("wiki", "Wiki pages"),
    ("baselines", "Baselines (named revisions)"),
    ("revisions", "Source repository revisions"),
    ("activities", "Activity streams"),
    ("folders", "Folders of documents and pages"),
    ("projects", "Projects and project groups"),
    ("users", "Users"),
    ("security", "Permissions, roles, licenses"),
    ("builds", "Builds"),
    ("session", "Session and transactions"),
];

/// A command by full name (`area.name`).
pub fn find(full_name: &str) -> Option<&'static Command> {
    let (area, name) = full_name.split_once('.')?;
    commands().iter().find(|c| c.area == area && c.name == name)
}

/// Run a command by name with JSON arguments.
pub async fn call(
    polarion: &polarion::Polarion,
    full_name: &str,
    args: &Args,
) -> Result<serde_json::Value> {
    let command = find(full_name)
        .ok_or_else(|| CommandError::invalid(format!("unknown command `{full_name}`")))?;
    (command.run)(polarion, args).await
}
