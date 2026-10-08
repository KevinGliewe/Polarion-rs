//! The `polarion-mcp` binary end to end: JSON-RPC lines on stdin against a mock SOAP server.

use std::io::Write;
use std::process::Stdio;

use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SESSION: &str = "/polarion/ws/services/SessionWebService";
const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
const URI: &str = "subterra:data-service:objects:/default/";

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
];

fn envelope(header: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{header}<soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>"#
    )
}

fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(envelope("", body), "text/xml; charset=utf-8")
}

/// Mock server that accepts a token login and expects exactly one logout.
async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(SESSION))
        .and(body_string_contains("logInWithToken>"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            envelope(
                r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">-1</ns1:sessionID></soapenv:Header>"#,
                "<logInWithTokenResponse/>",
            ),
            "text/xml",
        ).set_delay(std::time::Duration::from_millis(200)))
        // Slow on purpose: stdin closes while calls are still running, and they must finish.
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(SESSION))
        .and(body_string_contains("endSession>"))
        .respond_with(xml("<endSessionResponse/>"))
        .expect(1)
        .mount(&server)
        .await;
    server
}

async fn mock(server: &MockServer, needles: &[&str], body: &str) {
    let mut m = Mock::given(path(TRACKER));
    for n in needles {
        m = m.and(body_string_contains(*n));
    }
    m.respond_with(xml(body)).expect(1).mount(server).await;
}

/// Run a session at protocol 2025-06-18: send `requests` (numbered from 1), close stdin, return
/// the responses by id (the `initialize` response first).
async fn session(
    server: Option<&MockServer>,
    env: &[(&str, &str)],
    requests: &[Value],
) -> Vec<Value> {
    session_at("2025-06-18", server, env, requests).await
}

async fn session_at(
    version: &str,
    server: Option<&MockServer>,
    env: &[(&str, &str)],
    requests: &[Value],
) -> Vec<Value> {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion-mcp"));
    for var in VARS {
        cmd.env_remove(var);
    }
    if let Some(server) = server {
        cmd.env("POLARION_URL", format!("{}/polarion", server.uri()));
        cmd.env("POLARION_TOKEN", "t");
    }
    cmd.envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut input = String::new();
    input.push_str(
        &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}
        }})
        .to_string(),
    );
    input.push('\n');
    input.push_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    input.push('\n');
    for (i, request) in requests.iter().enumerate() {
        let mut request = request.clone();
        request["jsonrpc"] = json!("2.0");
        request["id"] = json!(i + 1);
        input.push_str(&request.to_string());
        input.push('\n');
    }
    let out = tokio::task::spawn_blocking(move || {
        let mut child = cmd.spawn().expect("binary runs");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert!(out.status.success());
    let mut responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    // One response per request (the notification has none); requests run concurrently, so
    // responses may come in any order.
    responses.sort_by_key(|r| r["id"].as_u64());
    assert_eq!(responses.len(), requests.len() + 1, "{responses:#?}");
    for (i, r) in responses.iter().enumerate() {
        assert_eq!(r["id"], json!(i), "{r}");
    }
    responses
}

fn call(tool: &str, arguments: Value) -> Value {
    json!({"method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

/// The text of a tool result, and whether it is an error.
fn tool_text(response: &Value) -> (String, bool) {
    let result = &response["result"];
    (
        result["content"][0]["text"].as_str().unwrap().to_owned(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_and_list_tools() {
    let r = session(
        None,
        &[("POLARION_PROJECT", "P")],
        &[
            json!({"method": "tools/list"}),
            json!({"method": "ping"}),
            json!({"method": "frobnicate/everything"}),
        ],
    )
    .await;
    let init = &r[0]["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "polarion-mcp");
    let instructions = init["instructions"].as_str().unwrap();
    assert!(instructions.contains("read-only"), "{instructions}");
    assert!(
        instructions.contains("default project is P"),
        "{instructions}"
    );

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 18, "{names:?}");
    for name in [
        "get_work_item",
        "query_work_items",
        "polarion_commands",
        "polarion_describe",
        "polarion_call",
    ] {
        assert!(names.contains(&name), "{name} missing: {names:?}");
    }
    let get = tools.iter().find(|t| t["name"] == "get_work_item").unwrap();
    assert_eq!(get["inputSchema"]["required"], json!(["item"]));
    assert_eq!(get["annotations"]["readOnlyHint"], true);

    assert_eq!(r[2]["result"], json!({}));
    assert_eq!(r[3]["error"]["code"], -32601);
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_versions_are_negotiated() {
    for version in ["2024-11-05", "2025-03-26", "2025-11-25"] {
        let r = session_at(version, None, &[], &[json!({"method": "ping"})]).await;
        assert_eq!(r[0]["result"]["protocolVersion"], version);
    }
    // An unknown version gets one the server speaks.
    let r = session_at("1999-01-01", None, &[], &[json!({"method": "ping"})]).await;
    let chosen = r[0]["result"]["protocolVersion"].as_str().unwrap();
    assert!(
        chosen.starts_with("20") && chosen != "1999-01-01",
        "{chosen}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_command_can_be_a_tool() {
    let r = session(
        None,
        &[("POLARION_MCP_TOOLS", "all")],
        &[json!({"method": "tools/list"})],
    )
    .await;
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 18 + polarion_commands::commands().len());
    let delete = tools.iter().find(|t| t["name"] == "items_delete").unwrap();
    assert_eq!(delete["annotations"]["readOnlyHint"], false);
    assert!(
        delete["description"]
            .as_str()
            .unwrap()
            .contains("Modifies data")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn curated_and_generic_tools_call_polarion() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>", "projectId>P<", ">P-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id><title>Brakes</title><type><id>requirement</id></type></getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    mock(
        &server,
        &["getWorkItemsCount>", ">project.id:P AND (type:task)<"],
        "<getWorkItemsCountResponse><getWorkItemsCountReturn>7</getWorkItemsCountReturn></getWorkItemsCountResponse>",
    )
    .await;

    let r = session(
        Some(&server),
        &[("POLARION_PROJECT", "P")],
        &[
            call("get_work_item", json!({"item": "P-1"})),
            call(
                "polarion_call",
                json!({"command": "items.count", "args": {"query": "type:task"}}),
            ),
            call("polarion_describe", json!({"command": "items.history"})),
            call("polarion_commands", json!({"area": "builds"})),
        ],
    )
    .await;

    let (text, error) = tool_text(&r[1]);
    assert!(!error, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"id": "P-1", "project": "P", "type": "requirement", "title": "Brakes", "uri": format!("{URI}P${{WorkItem}}P-1")})
    );
    assert_eq!(tool_text(&r[2]), ("7".to_owned(), false));

    let described: Value = serde_json::from_str(&tool_text(&r[3]).0).unwrap();
    assert_eq!(described["name"], "items.history");
    assert_eq!(described["write"], false);
    assert_eq!(described["inputSchema"]["type"], "object");

    let builds: Value = serde_json::from_str(&tool_text(&r[4]).0).unwrap();
    assert_eq!(builds[0]["name"], "builds.all");
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_are_tool_errors() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-9" unresolvable="true"/></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    let r = session(
        Some(&server),
        &[],
        &[
            call("get_work_item", json!({"item": "P/P-9"})),
            call("get_work_item", json!({})),
            call("polarion_call", json!({"command": "items.frobnicate"})),
            call("no_such_tool", json!({})),
        ],
    )
    .await;
    assert_eq!(
        tool_text(&r[1]),
        (
            "error (notFound): work item `P/P-9` not found".to_owned(),
            true
        )
    );
    let (text, error) = tool_text(&r[2]);
    assert!(error && text.starts_with("error (invalid):"), "{text}");
    assert!(text.contains("item"), "{text}");
    let (text, error) = tool_text(&r[3]);
    assert!(error && text.contains("unknown command"), "{text}");
    assert_eq!(r[4]["error"]["code"], -32602);
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_are_refused_without_write_projects() {
    let server = MockServer::start().await;
    let r = session(
        Some(&server),
        &[],
        &[call(
            "polarion_call",
            json!({"command": "items.delete", "args": {"item": "P/P-1"}}),
        )],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_go_through_with_write_projects() {
    let server = server().await;
    mock(
        &server,
        &["addAssignee>", "assigneeId>jdoe<"],
        "<addAssigneeResponse><addAssigneeReturn>true</addAssigneeReturn></addAssigneeResponse>",
    )
    .await;
    let r = session(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &[call(
            "polarion_call",
            json!({"command": "items.add-assignee", "args": {"item": "Sandbox/SB-1", "user": "jdoe"}}),
        )],
    )
    .await;
    assert!(
        r[0]["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Writes are enabled for: Sandbox.")
    );
    assert_eq!(tool_text(&r[1]), (r#"{"changed":true}"#.to_owned(), false));
}
