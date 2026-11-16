//! `polarion`: command-line access to Siemens Polarion ALM, generated from the
//! `polarion-commands` catalog.

mod cli;
mod render;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Parser as _;
use polarion_commands::{
    AnyCommand, CommandError, Context, Entry, LocalFiles, View, catalog, find,
};
use serde_json::{Value, json};

use crate::cli::{Opts, Top};

/// What every invocation needs besides the command.
struct Globals {
    json: bool,
    out: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let opts = Opts::parse();
    let config = opts.global.config();
    let globals = Globals {
        json: opts.global.json,
        out: opts.global.out.clone(),
    };
    let command = match opts.command {
        Top::Commands { area } => return list(&globals, area.as_deref()),
        Top::Run(cli) => AnyCommand::from(cli),
    };
    let entry = find(command.name()).expect("commands come from the catalog");
    let args = serde_json::to_value(&command).expect("commands serialize")["args"].take();
    let result = match config.connect() {
        Ok(polarion) => {
            // The paths on a command line are the user's own.
            let cx = Context::new(&polarion, config.project.as_deref())
                .with_files(LocalFiles::Unrestricted);
            let result = command.run(&cx).await;
            // Every login opens a session on the server (which may hold a license); end it.
            let _ = polarion.logout().await;
            result
        }
        Err(e) => Err(e),
    };
    match result {
        Ok(output) => {
            let value = serde_json::to_value(&output).expect("results serialize");
            print(&globals, entry, &args, &value)
        }
        Err(e) => fail(&globals, &e),
    }
}

fn print(globals: &Globals, entry: &Entry, args: &Value, value: &Value) -> ExitCode {
    if let (View::Bytes, Some(path)) = (entry.view, &globals.out) {
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
        render::text(value, entry.view)
    };
    if let (View::Text, Some(path), false) = (entry.view, &globals.out, globals.json) {
        // Text such as Markdown, saved as it would be printed.
        if let Err(e) = std::fs::write(path, &text) {
            return fail(
                globals,
                &CommandError::invalid(format!("cannot write {path}: {e}")),
            );
        }
        eprintln!("wrote {} bytes to {path}", text.len());
        return ExitCode::SUCCESS;
    }
    // Empty text (an empty description) is a result, not a missing one.
    if text.is_empty() && !matches!(entry.view, View::Text) {
        eprintln!("no results");
    }
    // A closed pipe (`polarion ... | head`) is not an error worth reporting.
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
    if let Some(note) = truncation_note(entry, args, value) {
        eprintln!("note: {note}");
    }
    ExitCode::SUCCESS
}

/// A hint when a limited query returned exactly `limit` results.
fn truncation_note(entry: &Entry, args: &Value, value: &Value) -> Option<String> {
    let schema = entry.input_schema();
    if schema["properties"].get("limit").is_none() || args.get("all") == Some(&Value::Bool(true)) {
        return None;
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(polarion::DEFAULT_LIMIT));
    (value.as_array()?.len() as u64 == limit)
        .then(|| format!("showing the first {limit} results; use --limit N or --all for more"))
}

fn list(globals: &Globals, area: Option<&str>) -> ExitCode {
    let selected: Vec<&Entry> = catalog()
        .iter()
        .filter(|e| area.is_none_or(|a| e.area == a))
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
            .map(|e| json!({"name": e.full_name(), "summary": e.summary, "write": e.write, "input": e.input_schema(), "output": e.output_schema()}))
            .collect();
        println!(
            "{}",
            render::json(&Value::Array(list), std::io::stdout().is_terminal())
        );
    } else {
        let rows: Vec<(String, bool, String)> = selected
            .iter()
            .map(|e| {
                (
                    format!("{} {}", e.area, e.name),
                    e.write,
                    e.summary.to_owned(),
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
