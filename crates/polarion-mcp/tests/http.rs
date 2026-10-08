//! The `polarion-mcp` binary over Streamable HTTP, against a mock SOAP server.

use std::io::{BufRead, BufReader};
use std::process::{ChildStderr, Stdio};

use reqwest::StatusCode;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SESSION: &str = "/polarion/ws/services/SessionWebService";
const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
const URI: &str = "subterra:data-service:objects:/default/";
const TOKEN: &str = "0123456789abcdef-test";

/// Every variable the binary reads, cleared so the caller's environment cannot leak in.
const VARS: &[&str] = &[
    "POLARION_URL",
    "POLARION_PROJECT",
    "POLARION_INSECURE",
    "POLARION_TOKEN",
    "POLARION_USER",
    "POLARION_PASSWORD",
    "POLARION_WRITE_PROJECTS",
    "POLARION_ALLOW_GLOBAL_WRITES",
    "POLARION_MCP_TOOLS",
    "POLARION_MCP_HTTP",
    "POLARION_MCP_AUTH_TOKEN",
    "POLARION_MCP_ALLOWED_ORIGINS",
];

fn envelope(header: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{header}<soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>"#
    )
}

fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(envelope("", body), "text/xml; charset=utf-8")
}

/// Mock Polarion that expects exactly one token login (and `logouts` logouts).
async fn polarion(logouts: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(SESSION))
        .and(body_string_contains("logInWithToken>"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(
                    envelope(
                        r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">-1</ns1:sessionID></soapenv:Header>"#,
                        "<logInWithTokenResponse/>",
                    ),
                    "text/xml",
                )
                // Long enough for concurrent first calls to overlap.
                .set_delay(std::time::Duration::from_millis(200)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(SESSION))
        .and(body_string_contains("endSession>"))
        .respond_with(xml("<endSessionResponse/>"))
        .expect(logouts)
        .mount(&server)
        .await;
    server
}

/// A running `polarion-mcp --http`, killed on drop.
struct Mcp {
    child: std::process::Child,
    stderr: BufReader<ChildStderr>,
    url: String,
    http: reqwest::Client,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(polarion: Option<&MockServer>, env: &[(&str, &str)]) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion-mcp"));
    for var in VARS {
        cmd.env_remove(var);
    }
    if let Some(server) = polarion {
        cmd.env("POLARION_URL", format!("{}/polarion", server.uri()));
        cmd.env("POLARION_TOKEN", "t");
    }
    cmd.envs(env.iter().copied())
        .args(["--http", "127.0.0.1:0"]);
    cmd
}

/// Start the server with the auth token and wait for its `listening on` line.
fn start(polarion: Option<&MockServer>, env: &[(&str, &str)]) -> Mcp {
    let mut child = command(polarion, env)
        .env("POLARION_MCP_AUTH_TOKEN", TOKEN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let url = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .unwrap_or_else(|| panic!("no address in {line:?}"))
        .to_owned();
    Mcp {
        child,
        stderr,
        url,
        http: reqwest::Client::new(),
    }
}

impl Mcp {
    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(body.to_string())
    }

    /// POST with the bearer token; the status and the JSON body (if any).
    async fn send(&self, body: &Value) -> (StatusCode, Option<Value>) {
        let response = self
            .post(body)
            .bearer_auth(TOKEN)
            .header("MCP-Protocol-Version", "2025-06-18")
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        let json = (!text.is_empty())
            .then(|| serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}")));
        (status, json)
    }

    /// A tool call; its text and whether it is an error.
    async fn call(&self, id: u64, tool: &str, arguments: Value) -> (String, bool) {
        let (status, json) = self
            .send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}))
            .await;
        assert_eq!(status, StatusCode::OK);
        let result = &json.unwrap()["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap_or(false),
        )
    }
}

fn ping() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})
}

#[test]
fn startup_needs_an_auth_token() {
    for env in [vec![], vec![("POLARION_MCP_AUTH_TOKEN", "short")]] {
        let out = command(None, &env).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains("POLARION_MCP_AUTH_TOKEN"), "{stderr}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_need_the_token_and_an_allowed_origin() {
    let mcp = start(
        None,
        &[("POLARION_MCP_ALLOWED_ORIGINS", "http://localhost:3000")],
    );

    let response = mcp.post(&ping()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
    let response = mcp
        .post(&ping())
        .bearer_auth("0123456789abcdef-wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    for (origin, status) in [
        ("http://evil.example", StatusCode::FORBIDDEN),
        ("http://localhost:3000", StatusCode::OK),
    ] {
        let response = mcp
            .post(&ping())
            .bearer_auth(TOKEN)
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{origin}");
    }

    let response = mcp
        .http
        .get(&mcp.url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let response = mcp
        .post(&ping())
        .bearer_auth(TOKEN)
        .header("MCP-Protocol-Version", "1999-01-01")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = mcp
        .http
        .post(&mcp.url)
        .bearer_auth(TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error(), "{response:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_allowed_hosts_are_served() {
    // Default: loopback names only (DNS rebinding).
    let mcp = start(None, &[]);
    let response = mcp
        .post(&ping())
        .bearer_auth(TOKEN)
        .header("Host", "polarion-mcp.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Behind a proxy: its host name.
    let mcp = start(
        None,
        &[("POLARION_MCP_ALLOWED_HOSTS", "polarion-mcp.example.com")],
    );
    let response = mcp
        .post(&ping())
        .bearer_auth(TOKEN)
        .header("Host", "polarion-mcp.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn clients_share_one_polarion_session() {
    let server = polarion(0).await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemById>"))
        .respond_with(xml(&format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id><title>Brakes</title></getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        )))
        .expect(2)
        .mount(&server)
        .await;
    let mcp = start(Some(&server), &[("POLARION_PROJECT", "P")]);

    let (status, init) = mcp
        .send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}
        }}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let instructions = init.unwrap()["result"]["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        instructions.contains("share one Polarion session"),
        "{instructions}"
    );
    let (status, body) = mcp
        .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    assert_eq!((status, body), (StatusCode::ACCEPTED, None));

    // Two clients at once: one login between them.
    let (a, b) = tokio::join!(
        mcp.call(1, "get_work_item", json!({"item": "P-1"})),
        mcp.call(2, "get_work_item", json!({"item": "P-1"})),
    );
    for (text, error) in [a, b] {
        assert!(!error, "{text}");
        let item: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(item["title"], "Brakes");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_are_refused_over_http() {
    let server = MockServer::start().await;
    let mcp = start(Some(&server), &[("POLARION_WRITE_PROJECTS", "Sandbox")]);
    let (text, error) = mcp
        .call(
            1,
            "polarion_call",
            json!({"command": "session.begin-transaction"}),
        )
        .await;
    assert!(
        error && text.contains("share one Polarion session"),
        "{text}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigint_ends_the_polarion_session() {
    let server = polarion(1).await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemsCount>"))
        .respond_with(xml(
            "<getWorkItemsCountResponse><getWorkItemsCountReturn>3</getWorkItemsCountReturn></getWorkItemsCountResponse>",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let mut mcp = start(Some(&server), &[("POLARION_PROJECT", "P")]);
    let (text, error) = mcp
        .call(1, "count_work_items", json!({"query": "type:task"}))
        .await;
    assert_eq!((text.as_str(), error), ("3", false));

    let pid = mcp.child.id().to_string();
    let status = std::process::Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap();
    assert!(status.success());
    let exit = tokio::task::spawn_blocking(move || {
        let exit = mcp.child.wait().unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut mcp.stderr, &mut rest).unwrap();
        (exit, rest)
    })
    .await
    .unwrap();
    assert!(exit.0.success(), "{exit:?}");
    assert!(exit.1.contains("shutting down"), "{}", exit.1);
}
