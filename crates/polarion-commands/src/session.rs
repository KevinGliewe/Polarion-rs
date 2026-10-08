//! `session.*`: session state and transactions.
//!
//! Transactions only make sense within one process (the MCP server, or a program using
//! this crate); every CLI invocation opens and ends its own session.

use polarion_commands_derive::cmd;

use crate::{Context, Done, Result, Run};

/// Log in (if needed) and report whether the session is valid.
#[cmd(view = scalar)]
pub struct Check {}

impl Run for Check {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        // Logging in again would open a second session and leave the first one behind.
        if !cx.polarion.has_session().await? {
            cx.polarion.login().await?;
        }
        Ok(cx.polarion.has_session().await?)
    }
}

/// Whether an explicit transaction is open.
#[cmd(view = scalar)]
pub struct InTransaction {}

impl Run for InTransaction {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.in_transaction().await?)
    }
}

/// Start a transaction (changes are applied together on commit)
#[cmd(write, view = done)]
pub struct BeginTransaction {}

impl Run for BeginTransaction {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion.begin_transaction().await?;
        Ok(Done)
    }
}

/// Commit the open transaction.
#[cmd(write, view = done)]
pub struct Commit {}

impl Run for Commit {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion.commit().await?;
        Ok(Done)
    }
}

/// Discard the open transaction.
#[cmd(write, view = done)]
pub struct Rollback {}

impl Run for Rollback {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion.rollback().await?;
        Ok(Done)
    }
}
