//! `polarion-mcp`: an MCP server giving AI agents access to Siemens Polarion ALM, over stdio
//! (default) or Streamable HTTP (`--http ADDR`).
//!
//! Configuration comes from the environment, as for the `polarion` command:
//! `POLARION_URL`, `POLARION_TOKEN` (or `POLARION_USER` and `POLARION_PASSWORD`),
//! `POLARION_PROJECT`, `POLARION_INSECURE`, and for writes `POLARION_WRITE_PROJECTS` /
//! `POLARION_ALLOW_GLOBAL_WRITES` (read-only without them). `POLARION_MCP_TOOLS=all`
//! registers every command as its own tool.

mod files;
mod http;
mod server;
mod stdio;

use std::process::ExitCode;

use polarion_commands::Config;

use crate::server::{PolarionMcp, Transport};

const USAGE: &str = "\
Usage: polarion-mcp [--http ADDR]

MCP server for Siemens Polarion ALM. Speaks stdio by default; with --http ADDR
(e.g. 127.0.0.1:8787) it serves Streamable HTTP at http://ADDR/mcp instead.

Environment:
  POLARION_URL, POLARION_TOKEN (or POLARION_USER + POLARION_PASSWORD), POLARION_PROJECT,
  POLARION_INSECURE, POLARION_WRITE_PROJECTS, POLARION_ALLOW_GLOBAL_WRITES
    POLARION_MCP_TOOLS=all          every command as its own tool
  POLARION_MCP_FILES=DIR          exchange folder: attachments and PDFs saved there (save_as),
                                  uploads read from there (data-file)

  POLARION_MCP_HTTP=ADDR          same as --http ADDR
  POLARION_MCP_AUTH_TOKEN         HTTP: required bearer token for clients (16+ characters)
  POLARION_MCP_ALLOWED_ORIGINS    HTTP: browser origins allowed to connect, with scheme and port
                                  (comma-separated, e.g. http://localhost:3000; default: none)
  POLARION_MCP_ALLOWED_HOSTS      HTTP: Host names the server answers to (comma-separated;
                                  default: localhost, 127.0.0.1, ::1); behind a proxy, its name
";

fn main() -> ExitCode {
    let mut http_addr = std::env::var("POLARION_MCP_HTTP")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--http" => match args.next() {
                Some(addr) => http_addr = Some(addr),
                None => return usage_error("--http needs an address, e.g. 127.0.0.1:8787"),
            },
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("polarion-mcp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => match other.strip_prefix("--http=") {
                Some(addr) => http_addr = Some(addr.to_owned()),
                None => return usage_error(&format!("unknown argument `{other}`")),
            },
        }
    }

    let config = Config::from_env();
    let all_tools = std::env::var("POLARION_MCP_TOOLS").is_ok_and(|v| v.trim() == "all");
    let exchange = match files::Exchange::from_env() {
        Ok(exchange) => exchange,
        Err(message) => {
            eprintln!("polarion-mcp: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Some(addr) = http_addr else {
        return match stdio::run(PolarionMcp::new(
            config,
            all_tools,
            Transport::Stdio,
            exchange,
        )) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("polarion-mcp: {message}");
                ExitCode::FAILURE
            }
        };
    };
    let settings = match http::Settings::from_env() {
        Ok(settings) => settings,
        Err(message) => {
            eprintln!("polarion-mcp: {message}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let server = PolarionMcp::new(config, all_tools, Transport::Http, exchange);
    match runtime.block_on(http::run(&addr, server, settings)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("polarion-mcp: cannot serve on {addr}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("polarion-mcp: {message}\n\n{USAGE}");
    ExitCode::from(2)
}
