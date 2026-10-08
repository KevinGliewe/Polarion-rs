//! The stdio transport: one JSON-RPC message per line on stdin, responses on stdout.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::server::Server;

/// Serve until stdin closes, then end the Polarion session.
pub fn run(server: &Server) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
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
