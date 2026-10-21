//! `polarion-mcp`: an MCP server giving AI agents access to Siemens Polarion ALM, over stdio
//! (default) or Streamable HTTP (`--http ADDR`).
//!
//! Configuration comes from the environment, as for the `polarion` command:
//! `POLARION_URL`, `POLARION_TOKEN` (or `POLARION_USER` and `POLARION_PASSWORD`),
//! `POLARION_PROJECT`, `POLARION_INSECURE`, and for writes `POLARION_WRITE_PROJECTS` /
//! `POLARION_ALLOW_GLOBAL_WRITES` (read-only without them) or `POLARION_WRITE_TARGETS` (only
//! the objects it names; set, even empty, it overrides the other two). Which tools are offered,
//! and which commands need their area's guide first, comes from the optional config file
//! (`--config FILE` or `POLARION_MCP_CONFIG`), `POLARION_MCP_TOOLS` and
//! `POLARION_MCP_REQUIRE_GUIDES`.

mod config;
mod files;
mod guides;
mod http;
mod server;
mod stdio;

use std::path::PathBuf;
use std::process::ExitCode;

use polarion_commands::Config;

use crate::config::Selection;
use crate::server::{PolarionMcp, Transport};

const USAGE: &str = "\
Usage: polarion-mcp [--http ADDR] [--config FILE]

MCP server for Siemens Polarion ALM. Speaks stdio by default; with --http ADDR
(e.g. 127.0.0.1:8787) it serves Streamable HTTP at http://ADDR/mcp instead.

The config file (TOML, optional) chooses the tools; every key is optional:
  tools = [\"default\", \"documents.*\"]   commands with a tool of their own (default: \"default\",
                                       the everyday reads, and the everyday writes (work items,
                                       documents, test runs, plans) when writes are enabled;
                                       \"all\": every command; without writes, those writes get
                                       no tool at all)
  disable = [\"*.delete\", \"wiki.*\"]     commands switched off everywhere (polarion_call too),
                                       or generic tools (polarion_call, ...) to leave out
  output_schemas = false               no outputSchema / structuredContent (smaller tool list)
  require_guides = \"write\"             commands refused until polarion_guide was called for their
                                       area (once per server process): \"write\" (default) the
                                       writes, when writes are enabled; \"all\" every command;
                                       \"none\" none. Areas: items -> work-items, documents, tests,
                                       plans, wiki; the others -> overview
Selectors are command names (items.get), tool names (get_work_item) or `default`/`all`;
`*` is a wildcard. The guides are also resources: polarion://guide/<area>.

Environment:
  POLARION_URL, POLARION_TOKEN (or POLARION_USER + POLARION_PASSWORD), POLARION_PROJECT,
  POLARION_INSECURE, POLARION_WRITE_PROJECTS, POLARION_ALLOW_GLOBAL_WRITES
  POLARION_WRITE_TARGETS=TARGETS  only these objects may be written (set, even empty, it
                                  overrides the two above): ID, PROJECT/ID, doc:SPACE/NAME,
                                  doc:PROJECT/SPACE/NAME, run:ID, run:PROJECT/ID or a URI,
                                  comma-separated (\\, for a comma in a name). Refused even for
                                  them: creates, deletes of whole objects, workflow actions,
                                  status and resolution changes, the body of a document that
                                  holds work items. Allowed: their attachments, work records
                                  and test records, deleting those included. Every object a
                                  write names (link targets, test cases, defects, plan items)
                                  must be a target too. Not prevented: what the server does in
                                  response (suspect links on work items that link to a target,
                                  notifications)
  POLARION_MCP_CONFIG=FILE        same as --config FILE
  POLARION_MCP_TOOLS=SELECTORS    the config file's `tools`, comma-separated (e.g. all); wins
                                  over the file
  POLARION_MCP_REQUIRE_GUIDES=X   the config file's `require_guides` (write, all, none); wins
                                  over the file
  POLARION_MCP_FILES=DIR          exchange folder: attachments and PDFs saved there (save_as),
                                  uploads read from there (data-file)

  POLARION_MCP_HTTP=ADDR          same as --http ADDR
  POLARION_MCP_AUTH_TOKEN         HTTP: required token for clients (16+ characters), sent as
                                  Authorization: Bearer <token>, or in the URL: /mcp/<token>
  POLARION_MCP_ALLOW_UNAUTHENTICATED=1
                                  HTTP: no token (instead of POLARION_MCP_AUTH_TOKEN); every
                                  client that reaches the port acts as the Polarion account
  POLARION_MCP_ALLOWED_ORIGINS    HTTP: browser origins allowed to connect, with scheme and port
                                  (comma-separated, e.g. http://localhost:3000; default: none)
  POLARION_MCP_ALLOWED_HOSTS      HTTP: Host names the server answers to (comma-separated;
                                  default: localhost, 127.0.0.1, ::1); behind a proxy, its name
";

fn main() -> ExitCode {
    let mut http_addr = std::env::var("POLARION_MCP_HTTP")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let mut config_file = std::env::var_os("POLARION_MCP_CONFIG")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--http" => match args.next() {
                Some(addr) => http_addr = Some(addr),
                None => return usage_error("--http needs an address, e.g. 127.0.0.1:8787"),
            },
            "--config" => match args.next() {
                Some(file) => config_file = Some(file.into()),
                None => return usage_error("--config needs a file"),
            },
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("polarion-mcp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => {
                if let Some(addr) = other.strip_prefix("--http=") {
                    http_addr = Some(addr.to_owned());
                } else if let Some(file) = other.strip_prefix("--config=") {
                    config_file = Some(file.into());
                } else {
                    return usage_error(&format!("unknown argument `{other}`"));
                }
            }
        }
    }

    let config = Config::from_env();
    // A write allowlist that does not parse is not served at all.
    if let Err(e) = config.resolve_write_targets() {
        eprintln!("polarion-mcp: {e}");
        return ExitCode::FAILURE;
    }
    let tools = std::env::var("POLARION_MCP_TOOLS")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let guides = std::env::var("POLARION_MCP_REQUIRE_GUIDES")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let selection = match Selection::load(
        config_file.as_deref(),
        tools.as_deref(),
        guides.as_deref(),
        config.writes_enabled(),
    ) {
        Ok(selection) => selection,
        Err(message) => {
            eprintln!("polarion-mcp: {message}");
            return ExitCode::FAILURE;
        }
    };
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
            selection,
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
    let server = PolarionMcp::new(config, selection, Transport::Http, exchange);
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
