//! `polarion`: command-line access to Siemens Polarion ALM, generated from the
//! `polarion-commands` catalog.

mod cli;
mod render;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use polarion_commands::{Args, CommandError, Output, commands};
use serde_json::{Value, json};

use crate::cli::{Globals, Invocation};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let matches = cli::build().get_matches();
    let globals = cli::globals(&matches);
    let invocation = match cli::invocation(&matches) {
        Ok(i) => i,
        Err(message) => return fail(&globals, &CommandError::invalid(message)),
    };
    match invocation {
        Invocation::List(area) => list(&globals, area.as_deref()),
        Invocation::Run { command, args } => {
            let args = Args::new(args, globals.config.project.clone());
            let result = match globals.config.connect() {
                Ok(polarion) => {
                    let result = (command.run)(&polarion, &args).await;
                    // Every login opens a session on the server (which may hold a license); end it.
                    let _ = polarion.logout().await;
                    result
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(value) => print(&globals, command, &args, &value),
                Err(e) => fail(&globals, &e),
            }
        }
    }
}

fn print(
    globals: &Globals,
    command: &polarion_commands::Command,
    args: &Args,
    value: &Value,
) -> ExitCode {
    if let (Output::Bytes, Some(path)) = (command.output, &globals.out) {
        let data = value
            .get("base64")
            .and_then(Value::as_str)
            .and_then(|b| BASE64.decode(b).ok())
            .unwrap_or_default();
        if let Err(e) = std::fs::write(path, &data) {
            return fail(
                globals,
                &CommandError::invalid(format!("cannot write {path}: {e}")),
            );
        }
        eprintln!("wrote {} bytes to {path}", data.len());
        if !globals.json {
            return ExitCode::SUCCESS;
        }
    }
    let text = if globals.json {
        render::json(value, std::io::stdout().is_terminal()) + "\n"
    } else {
        render::text(value, command.output)
    };
    if text.is_empty() {
        eprintln!("no results");
    }
    // A closed pipe (`polarion ... | head`) is not an error worth reporting.
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
    if let Some(note) = truncation_note(command, args, value) {
        eprintln!("note: {note}");
    }
    ExitCode::SUCCESS
}

/// A hint when a limited query returned exactly `limit` results.
fn truncation_note(
    command: &polarion_commands::Command,
    args: &Args,
    value: &Value,
) -> Option<String> {
    let has_limit = command.params.iter().any(|p| p.name == "limit");
    if !has_limit || args.bool("all").unwrap_or(false) {
        return None;
    }
    let limit = args
        .opt_u32("limit")
        .ok()
        .flatten()
        .unwrap_or(polarion::DEFAULT_LIMIT);
    (value.as_array()?.len() == limit as usize)
        .then(|| format!("showing the first {limit} results; use --limit N or --all for more"))
}

fn list(globals: &Globals, area: Option<&str>) -> ExitCode {
    let selected: Vec<_> = commands()
        .iter()
        .filter(|c| area.is_none_or(|a| c.area == a))
        .collect();
    if selected.is_empty() {
        return fail(
            globals,
            &CommandError::invalid(format!("unknown area `{}`", area.unwrap_or_default())),
        );
    }
    if globals.json {
        let list: Vec<Value> = selected
            .iter()
            .map(|c| json!({"name": c.full_name(), "summary": c.summary, "write": c.write, "input": c.input_schema()}))
            .collect();
        println!(
            "{}",
            render::json(&Value::Array(list), std::io::stdout().is_terminal())
        );
    } else {
        let rows: Vec<(String, bool, String)> = selected
            .iter()
            .map(|c| {
                (
                    format!("{} {}", c.area, c.name),
                    c.write,
                    c.summary.to_owned(),
                )
            })
            .collect();
        print!("{}", render::commands(&rows));
    }
    ExitCode::SUCCESS
}

fn fail(globals: &Globals, e: &CommandError) -> ExitCode {
    if globals.json {
        eprintln!(
            "{}",
            json!({"error": {"code": e.code, "message": e.message}})
        );
    } else {
        eprintln!("error: {}", e.message);
    }
    ExitCode::from(e.exit_code())
}
