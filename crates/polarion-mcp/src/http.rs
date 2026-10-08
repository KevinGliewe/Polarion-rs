//! The Streamable HTTP transport: rmcp's service at `/mcp`, behind a bearer token.
//!
//! Stateless (no `Mcp-Session-Id`) with JSON replies: every tool call is a plain request/response.
//! All clients share the server's Polarion session.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::server::PolarionMcp;

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
    /// `Host` values the server answers to; `None`: rmcp's default (loopback names).
    allowed_hosts: Option<Vec<String>>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("allowed_origins", &self.allowed_origins)
            .field("allowed_hosts", &self.allowed_hosts)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// `POLARION_MCP_AUTH_TOKEN` (required), `POLARION_MCP_ALLOWED_ORIGINS` and
    /// `POLARION_MCP_ALLOWED_HOSTS`.
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
        let list = |var: &str| {
            std::env::var(var)
                .ok()
                .map(|v| polarion_commands::split_list(&v))
                .filter(|l| !l.is_empty())
        };
        Ok(Self {
            auth_token,
            allowed_origins: list("POLARION_MCP_ALLOWED_ORIGINS").unwrap_or_default(),
            allowed_hosts: list("POLARION_MCP_ALLOWED_HOSTS"),
        })
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .is_some_and(|(_, token)| {
                constant_time_eq(token.trim().as_bytes(), self.auth_token.as_bytes())
            })
    }

    /// rmcp's settings: stateless, JSON replies, Host and Origin validation.
    fn rmcp_config(&self, cancel: CancellationToken) -> StreamableHttpServerConfig {
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_max_request_body_bytes(BODY_LIMIT)
            .with_cancellation_token(cancel)
            // Browsers (DNS rebinding) only from the listed origins; none by default.
            .with_allowed_origins(self.allowed_origins.clone())
            .enforce_origin_validation();
        match &self.allowed_hosts {
            Some(hosts) => config.with_allowed_hosts(hosts.clone()),
            None => config,
        }
    }
}

/// Serve on `addr` until Ctrl-C or SIGTERM, then end the Polarion session.
pub async fn run(addr: &str, server: PolarionMcp, settings: Settings) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "polarion-mcp listening on http://{}/mcp ({})",
        listener.local_addr()?,
        server.access()
    );
    let cancel = CancellationToken::new();
    let handler = server.clone();
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        settings.rmcp_config(cancel.child_token()),
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(from_fn_with_state(Arc::new(settings), authorize));
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            cancel.cancel();
        })
        .await?;
    eprintln!("polarion-mcp: shutting down");
    server.shutdown().await;
    Ok(())
}

/// Only requests with the bearer token reach the MCP service.
async fn authorize(
    State(settings): State<Arc<Settings>>,
    request: Request,
    next: Next,
) -> Response {
    if settings.authorized(request.headers()) {
        return next.run(request).await;
    }
    let mut response = (
        StatusCode::UNAUTHORIZED,
        "missing or invalid bearer token\n",
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
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
