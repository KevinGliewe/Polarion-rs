//! `polarion`: read-only command-line access to Siemens Polarion ALM.

mod cli;
mod render;
mod run;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use crate::cli::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run::run(&cli).await {
        Ok(output) => {
            let text = if cli.json {
                render::json(&output.value, std::io::stdout().is_terminal()) + "\n"
            } else {
                render::text(&output.value, output.view)
            };
            if text.is_empty() {
                eprintln!("no results");
            }
            // A closed pipe (`polarion ... | head`) is not an error worth reporting.
            let _ = std::io::stdout().lock().write_all(text.as_bytes());
            if let Some(note) = output.note {
                eprintln!("note: {note}");
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            if cli.json {
                let error =
                    json!({"error": {"code": failure.code(), "message": failure.message()}});
                eprintln!("{error}");
            } else {
                eprintln!("error: {}", failure.message());
            }
            ExitCode::from(failure.exit_code())
        }
    }
}
