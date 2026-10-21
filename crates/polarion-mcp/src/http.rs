//! The Streamable HTTP transport: rmcp's service at `/mcp`, behind a token.
//!
//! Clients send the token as `Authorization: Bearer <token>` or in the URL, `/mcp/<token>` (for
//! clients that cannot set headers); `POLARION_MCP_ALLOW_UNAUTHENTICATED` turns the check off.
//!
//! Stateless (no `Mcp-Session-Id`) with JSON replies: every tool call is a plain request/response.
//! All clients share the server's Polarion session.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::{Router, ServiceExt};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;
use tower_layer::Layer;

use crate::server::PolarionMcp;

/// Largest accepted request (base64 attachments).
const BODY_LIMIT: usize = 32 * 1024 * 1024;

/// Shortest accepted token.
const MIN_TOKEN_LEN: usize = 16;

/// The MCP endpoint.
const PATH: &str = "/mcp";

/// Access rules for HTTP clients.
#[derive(Clone)]
pub struct Settings {
    /// Clients send `Authorization: Bearer <token>` or request `/mcp/<token>`; `None`: no
    /// authentication (`POLARION_MCP_ALLOW_UNAUTHENTICATED`).
    auth_token: Option<String>,
    /// Browser origins allowed to call the server (requests without `Origin` are not browsers).
    allowed_origins: Vec<String>,
    /// `Host` values the server answers to; `None`: rmcp's default (loopback names).
    allowed_hosts: Option<Vec<String>>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("authenticated", &self.auth_token.is_some())
            .field("allowed_origins", &self.allowed_origins)
            .field("allowed_hosts", &self.allowed_hosts)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// `POLARION_MCP_AUTH_TOKEN` (required unless `POLARION_MCP_ALLOW_UNAUTHENTICATED` is set,
    /// and refused with it), `POLARION_MCP_ALLOWED_ORIGINS` and `POLARION_MCP_ALLOWED_HOSTS`.
    pub fn from_env() -> Result<Self, String> {
        let auth_token = std::env::var("POLARION_MCP_AUTH_TOKEN")
            .map(|t| t.trim().to_owned())
            .unwrap_or_default();
        let unauthenticated = std::env::var("POLARION_MCP_ALLOW_UNAUTHENTICATED")
            .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"));
        let auth_token = match (unauthenticated, auth_token.is_empty()) {
            (true, true) => None,
            (true, false) => {
                return Err(
                    "POLARION_MCP_ALLOW_UNAUTHENTICATED and POLARION_MCP_AUTH_TOKEN are \
                     both set: unset the token to accept every client, or the other to require it"
                        .to_owned(),
                );
            }
            (false, true) => {
                return Err(
                    "set POLARION_MCP_AUTH_TOKEN: HTTP clients must authenticate with it \
                     (Authorization: Bearer <token>, or the URL /mcp/<token>); \
                     POLARION_MCP_ALLOW_UNAUTHENTICATED=1 accepts every client instead"
                        .to_owned(),
                );
            }
            (false, false) if auth_token.chars().count() < MIN_TOKEN_LEN => {
                return Err(format!(
                    "POLARION_MCP_AUTH_TOKEN must be at least {MIN_TOKEN_LEN} characters"
                ));
            }
            (false, false) => Some(auth_token),
        };
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

    /// The header carries the token (or none is needed).
    fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = &self.auth_token else {
            return true;
        };
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .is_some_and(|(_, token)| {
                constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
            })
    }

    /// `segment` (percent-encoded, from `/mcp/<segment>`) is the token.
    fn token_in_path(&self, segment: &str) -> bool {
        self.auth_token.as_ref().is_some_and(|expected| {
            percent_decode(segment)
                .is_some_and(|token| constant_time_eq(&token, expected.as_bytes()))
        })
    }

    /// How clients authenticate, for the startup line.
    fn describe(&self) -> &'static str {
        if self.auth_token.is_some() {
            "token: Authorization: Bearer, or the URL /mcp/<token>"
        } else {
            "NO AUTHENTICATION: every client that reaches it acts as the Polarion account"
        }
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
        "polarion-mcp listening on http://{}{PATH} ({}; {})",
        listener.local_addr()?,
        server.access(),
        settings.describe()
    );
    let cancel = CancellationToken::new();
    let handler = server.clone();
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        settings.rmcp_config(cancel.child_token()),
    );
    let router = Router::new().nest_service(PATH, service);
    // Around the router, not inside it: `authorize` rewrites `/mcp/<token>` before routing.
    let app = from_fn_with_state(Arc::new(settings), authorize).layer(router);
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            cancel.cancel();
        })
        .await?;
    eprintln!("polarion-mcp: shutting down");
    server.shutdown().await;
    Ok(())
}

/// Only requests with the token reach the MCP service: `/mcp` (or `/mcp/`) with the header, or
/// `/mcp/<token>` (or `/mcp/<token>/`), which is served as `/mcp`. Without authentication,
/// `/mcp` only.
async fn authorize(
    State(settings): State<Arc<Settings>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(rest) = request.uri().path().strip_prefix(PATH) else {
        // Not the endpoint: the router answers 404.
        return next.run(request).await;
    };
    let allowed = match rest.strip_prefix('/') {
        // `/mcpx`: not the endpoint.
        None if !rest.is_empty() => return next.run(request).await,
        None => settings.authorized(request.headers()),
        Some("") => settings.authorized(request.headers()),
        Some(segment) => {
            let segment = segment.strip_suffix('/').unwrap_or(segment);
            !segment.is_empty() && !segment.contains('/') && settings.token_in_path(segment)
        }
    };
    if !allowed {
        return unauthorized();
    }
    if request.uri().path() != PATH {
        *request.uri_mut() = endpoint(request.uri());
    }
    next.run(request).await
}

/// `uri` with the path `/mcp`, the query kept.
fn endpoint(uri: &Uri) -> Uri {
    let path_and_query = match uri.query() {
        Some(query) => format!("{PATH}?{query}"),
        None => PATH.to_owned(),
    };
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(path_and_query.parse().expect("a valid path"));
    Uri::from_parts(parts).expect("a valid URI")
}

fn unauthorized() -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "missing or invalid token\n").into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

/// Decode `%XX` escapes; `None` for a malformed one.
fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
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
    fn path_tokens_are_percent_decoded() {
        assert_eq!(
            percent_decode("a%20b%C3%BC").unwrap(),
            "a b\u{fc}".as_bytes()
        );
        assert_eq!(percent_decode("plain").unwrap(), b"plain");
        assert!(percent_decode("bad%2").is_none());
        assert!(percent_decode("bad%zz").is_none());
        assert!(percent_decode("bad%+1").is_none());
    }

    #[test]
    fn tokens_compare_exactly() {
        assert!(constant_time_eq(b"secret-token-123", b"secret-token-123"));
        assert!(!constant_time_eq(b"secret-token-123", b"secret-token-124"));
        assert!(!constant_time_eq(b"secret", b"secret-token-123"));
    }
}
