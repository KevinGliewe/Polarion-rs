//! The Streamable HTTP transport: JSON-RPC messages POSTed to `/mcp`, answered with JSON.
//!
//! Stateless (no `Mcp-Session-Id`) and without server-initiated SSE streams: every tool call is
//! a plain request/response. All clients share the server's Polarion session.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};

use crate::server::{PROTOCOL_VERSIONS, Server};

/// Largest accepted request (base64 attachments).
const BODY_LIMIT: usize = 32 * 1024 * 1024;

/// Shortest accepted bearer token.
const MIN_TOKEN_LEN: usize = 16;

/// Access rules for HTTP clients.
#[derive(Clone)]
pub struct Settings {
    /// Clients send `Authorization: Bearer <auth_token>`.
    auth_token: String,
    /// Browser origins allowed to call the server (requests without `Origin` are not browsers).
    allowed_origins: Vec<String>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("allowed_origins", &self.allowed_origins)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// `POLARION_MCP_AUTH_TOKEN` (required) and `POLARION_MCP_ALLOWED_ORIGINS`.
    pub fn from_env() -> Result<Self, String> {
        let auth_token = std::env::var("POLARION_MCP_AUTH_TOKEN")
            .map(|t| t.trim().to_owned())
            .unwrap_or_default();
        if auth_token.is_empty() {
            return Err(
                "set POLARION_MCP_AUTH_TOKEN: HTTP clients must authenticate with it \
                        (Authorization: Bearer <token>)"
                    .to_owned(),
            );
        }
        if auth_token.chars().count() < MIN_TOKEN_LEN {
            return Err(format!(
                "POLARION_MCP_AUTH_TOKEN must be at least {MIN_TOKEN_LEN} characters"
            ));
        }
        let allowed_origins = std::env::var("POLARION_MCP_ALLOWED_ORIGINS")
            .map(|v| polarion_commands::split_list(&v))
            .unwrap_or_default();
        Ok(Self {
            auth_token,
            allowed_origins,
        })
    }

    /// The response to send instead when a request may not reach the server.
    fn reject(&self, headers: &HeaderMap) -> Option<Response> {
        // DNS rebinding: a browser page must not reach a server on localhost.
        if let Some(origin) = headers.get(header::ORIGIN) {
            let allowed = origin
                .to_str()
                .is_ok_and(|o| self.allowed_origins.iter().any(|a| a == o));
            if !allowed {
                return Some(error(StatusCode::FORBIDDEN, -32600, "origin not allowed"));
            }
        }
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());
        if !token.is_some_and(|t| constant_time_eq(t.as_bytes(), self.auth_token.as_bytes())) {
            let mut response = error(
                StatusCode::UNAUTHORIZED,
                -32600,
                "missing or invalid bearer token",
            );
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            return Some(response);
        }
        if let Some(version) = headers.get("mcp-protocol-version") {
            if !version
                .to_str()
                .is_ok_and(|v| PROTOCOL_VERSIONS.contains(&v))
            {
                return Some(error(
                    StatusCode::BAD_REQUEST,
                    -32600,
                    &format!(
                        "unsupported MCP-Protocol-Version (supported: {})",
                        PROTOCOL_VERSIONS.join(", ")
                    ),
                ));
            }
        }
        None
    }
}

struct App {
    server: Server,
    settings: Settings,
}

/// Serve on `addr` until Ctrl-C or SIGTERM, then end the Polarion session.
pub async fn run(addr: &str, server: Server, settings: Settings) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "polarion-mcp listening on http://{}/mcp ({})",
        listener.local_addr()?,
        server.access()
    );
    let app = Arc::new(App { server, settings });
    let router = Router::new()
        .route("/mcp", post(handle))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(Arc::clone(&app));
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    eprintln!("polarion-mcp: shutting down");
    app.server.shutdown().await;
    Ok(())
}

async fn handle(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = app.settings.reject(&headers) {
        return response;
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(e) => {
            return error(
                StatusCode::BAD_REQUEST,
                -32700,
                &format!("parse error: {e}"),
            );
        }
    };
    let reply = match message {
        // A batch (protocol 2025-03-26).
        Value::Array(batch) => {
            let mut replies = Vec::new();
            for message in &batch {
                replies.extend(app.server.handle(message).await);
            }
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        message => app.server.handle(&message).await,
    };
    match reply {
        Some(reply) => (
            [(header::CONTENT_TYPE, "application/json")],
            reply.to_string(),
        )
            .into_response(),
        // Notifications and client responses.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// An HTTP error with a JSON-RPC error body.
fn error(status: StatusCode, code: i64, message: &str) -> Response {
    let body = json!({"jsonrpc": "2.0", "id": null, "error": {"code": code, "message": message}});
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Compare without revealing where the first difference is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_exactly() {
        assert!(constant_time_eq(b"secret-token-123", b"secret-token-123"));
        assert!(!constant_time_eq(b"secret-token-123", b"secret-token-124"));
        assert!(!constant_time_eq(b"secret", b"secret-token-123"));
    }
}
