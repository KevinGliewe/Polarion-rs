//! The stdio transport: newline-delimited JSON-RPC on stdin and stdout (rmcp).

use rmcp::ServiceExt;

use crate::server::PolarionMcp;

/// Serve until stdin closes, then end the Polarion session.
pub fn run(server: PolarionMcp) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let served = match server.clone().serve(rmcp::transport::stdio()).await {
            Ok(running) => running.waiting().await.map(drop).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        server.shutdown().await;
        served
    })
}
