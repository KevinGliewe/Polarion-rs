//! `polarion-mcp`: an MCP server (stdio) giving AI agents access to Siemens Polarion ALM.
//!
//! Configuration comes from the environment, as for the `polarion` command:
//! `POLARION_URL`, `POLARION_TOKEN` (or `POLARION_USER` and `POLARION_PASSWORD`),
//! `POLARION_PROJECT`, `POLARION_INSECURE`, and for writes `POLARION_WRITE_PROJECTS` /
//! `POLARION_ALLOW_GLOBAL_WRITES` (read-only without them). `POLARION_MCP_TOOLS=all`
//! registers every command as its own tool.

mod server;

use std::io::{BufRead, Write};

use polarion_commands::Config;
use serde_json::{Value, json};

use crate::server::Server;

fn main() {
    let config = Config::from_env();
    let all_tools = std::env::var("POLARION_MCP_TOOLS").is_ok_and(|v| v.trim() == "all");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let server = Server::new(config, all_tools);

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => runtime.block_on(server.handle(&message)),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")},
            })),
        };
        if let Some(response) = response {
            let mut text = response.to_string();
            text.push('\n');
            if stdout
                .write_all(text.as_bytes())
                .and_then(|()| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
    runtime.block_on(server.shutdown());
}
