//! `session.*`: session state and transactions.
//!
//! Transactions only make sense within one process (the MCP server, or a program using
//! this crate); every CLI invocation opens and ends its own session.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "session",
        "check",
        false,
        Output::Scalar,
        "Log in (if needed) and report whether the session is valid",
        [],
        check
    ),
    command!(
        "session",
        "in-transaction",
        false,
        Output::Scalar,
        "Whether an explicit transaction is open",
        [],
        in_transaction
    ),
    command!(
        "session",
        "begin-transaction",
        true,
        Output::Done,
        "Start a transaction (changes are applied together on commit)",
        [],
        begin_transaction
    ),
    command!(
        "session",
        "commit",
        true,
        Output::Done,
        "Commit the open transaction",
        [],
        commit
    ),
    command!(
        "session",
        "rollback",
        true,
        Output::Done,
        "Discard the open transaction",
        [],
        rollback
    ),
];

async fn check(p: &Polarion, _: &Args) -> Result<Value> {
    // Logging in again would open a second session and leave the first one behind.
    if !p.has_session().await? {
        p.login().await?;
    }
    out(p.has_session().await?)
}

async fn in_transaction(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.in_transaction().await?)
}

async fn begin_transaction(p: &Polarion, _: &Args) -> Result<Value> {
    p.begin_transaction().await?;
    Ok(Value::Null)
}

async fn commit(p: &Polarion, _: &Args) -> Result<Value> {
    p.commit().await?;
    Ok(Value::Null)
}

async fn rollback(p: &Polarion, _: &Args) -> Result<Value> {
    p.rollback().await?;
    Ok(Value::Null)
}
