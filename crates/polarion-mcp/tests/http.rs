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
    "POLARION_WRITE_TARGETS",
    "POLARION_MCP_TOOLS",
    "POLARION_MCP_CONFIG",
    "POLARION_MCP_REQUIRE_GUIDES",
    "POLARION_MCP_HTTP",
    "POLARION_MCP_AUTH_TOKEN",
    "POLARION_MCP_ALLOWED_ORIGINS",
    "POLARION_MCP_FILES",
    "POLARION_MCP_ALLOWED_HOSTS",
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
    /// Kept open so the server can write to it; read by the Unix-only signal test.
    #[cfg_attr(not(unix), allow(dead_code))]
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

#[tokio::test(flavor = "multi_thread")]
async fn write_targets_restrict_writes_over_http() {
    let server = MockServer::start().await;
    let mcp = start(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1"),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
            // The library's refusals: the gate is off.
            ("POLARION_MCP_REQUIRE_GUIDES", "none"),
        ],
    );
    // Another item, and the target's id in another project.
    let (text, error) = mcp
        .call(
            1,
            "polarion_call",
            json!({"command": "items.add-assignee", "args": {"item": "Sandbox/SB-2", "user": "jdoe"}}),
        )
        .await;
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    let (text, error) = mcp
        .call(
            2,
            "polarion_call",
            json!({"command": "items.add-assignee", "args": {"item": "Other/SB-1", "user": "jdoe"}}),
        )
        .await;
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    // The same through the Markdown write's tool of its own.
    let (text, error) = mcp
        .call(
            3,
            "set_work_item_description_markdown",
            json!({"item": "Sandbox/SB-2", "markdown": "# Brakes"}),
        )
        .await;
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

/// The curated write tools, in their order: listed only with writes enabled.
const WRITE_TOOLS: &[&str] = &[
    "create_work_item",
    "update_work_item",
    "set_work_item_description_markdown",
    "append_work_item_description_markdown",
    "add_work_item_comment",
    "add_work_item_link",
    "remove_work_item_link",
    "add_work_item_attachment",
    "create_document",
    "update_document",
    "add_document_comment",
    "create_test_run",
    "update_test_run",
    "add_test_record",
    "set_test_steps",
    "create_plan",
    "update_plan",
    "add_plan_items",
    "remove_plan_items",
];

#[tokio::test(flavor = "multi_thread")]
async fn write_tools_are_listed_over_http_only_with_writes() {
    let cases: [(&[(&str, &str)], bool); 3] = [
        (&[], false),
        (&[("POLARION_WRITE_PROJECTS", "Sandbox")], true),
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "SB-1"),
            ],
            true,
        ),
    ];
    for (env, writes) in cases {
        let mcp = start(None, env);
        let (status, json) = mcp
            .send(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .await;
        assert_eq!(status, StatusCode::OK);
        let json = json.unwrap();
        let names: Vec<&str> = json["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // 25 curated reads, the 19 curated writes with writes enabled, 4 generic tools.
        assert_eq!(names.len(), if writes { 48 } else { 29 }, "{names:?}");
        assert_eq!(names[24], "get_user", "{names:?}");
        if writes {
            assert_eq!(names[25..44], *WRITE_TOOLS, "{names:?}");
        }
        assert_eq!(
            names[names.len() - 4..],
            [
                "polarion_guide",
                "polarion_commands",
                "polarion_describe",
                "polarion_call"
            ]
        );
        assert!(
            WRITE_TOOLS.iter().all(|t| names.contains(t) == writes),
            "{names:?}"
        );
    }
}

/// Over HTTP the guides read count for the server process: once one client read an area's
/// guide, the area's writes pass the gate for every client.
#[tokio::test(flavor = "multi_thread")]
async fn a_guide_read_by_one_client_counts_for_all() {
    let server = polarion(0).await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("addAssignee>"))
        .and(body_string_contains("assigneeId>jdoe<"))
        .respond_with(xml(
            "<addAssigneeResponse><addAssigneeReturn>true</addAssigneeReturn></addAssigneeResponse>",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let mcp = start(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
        ],
    );
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
        instructions.contains(
            "the writes of an area are refused until its guide was read on this server (by any \
             of its clients)."
        ),
        "{instructions}"
    );
    let assign = json!({"command": "items.add-assignee", "args": {"item": "SB-1", "user": "jdoe"}});
    let (text, error) = mcp.call(1, "polarion_call", assign.clone()).await;
    assert!(
        error && text.starts_with("error (guideRequired):") && text.contains("\"work-items\""),
        "{text}"
    );
    // Another request (each is a client of its own here): the guide.
    let (text, error) = mcp
        .call(2, "polarion_guide", json!({"area": "work-items"}))
        .await;
    assert!(!error && text.starts_with("# Work items"), "{text}");
    let (text, error) = mcp.call(3, "polarion_call", assign).await;
    assert_eq!((text.as_str(), error), (r#"{"changed":true}"#, false));
    // The resources over HTTP too.
    let (status, list) = mcp
        .send(&json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list.unwrap()["result"]["resources"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
}

#[test]
fn the_startup_line_names_the_write_targets() {
    let mut child = command(
        None,
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1, doc:Specs/SRS"),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ("POLARION_MCP_AUTH_TOKEN", TOKEN),
        ],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .expect("binary runs");
    let mut line = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        line.contains("(writes restricted to work item Sandbox/SB-1, document Sandbox/Specs/SRS)"),
        "{line}"
    );
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
