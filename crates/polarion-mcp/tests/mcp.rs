//! The `polarion-mcp` binary end to end: JSON-RPC lines on stdin against a mock SOAP server.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
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
    "POLARION_MCP_FILES",
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
    assert_eq!(tools.len(), 18 + polarion_commands::catalog().len());
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

// ---- attachments

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const PDF: &[u8] = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n";

/// A work item attachment the mock serves once.
async fn attachment(server: &MockServer, id: &str, data: &[u8]) {
    mock(
        server,
        &["getAttachment>", &format!("attachmentId>{id}<")],
        &format!(
            "<getAttachmentResponse><getAttachmentReturn>{}</getAttachmentReturn></getAttachmentResponse>",
            BASE64.encode(data)
        ),
    )
    .await;
}

fn download(attachment: &str, extra: Value) -> Value {
    let mut args = json!({"item": "P/P-1", "attachment": attachment});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    call(
        "polarion_call",
        json!({"command": "items.attachment", "args": args}),
    )
}

/// A fresh directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("polarion-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_come_back_as_images_or_text() {
    let server = server().await;
    attachment(&server, "a-png", PNG).await;
    attachment(&server, "a-txt", b"hello\nworld").await;
    attachment(&server, "a-pdf", PDF).await;
    let r = session(
        Some(&server),
        &[],
        &[
            download("a-png", json!({})),
            download("a-txt", json!({})),
            download("a-pdf", json!({})),
            // Refused before anything is downloaded.
            download("a-pdf", json!({"save_as": "spec.pdf"})),
        ],
    )
    .await;

    let image = &r[1]["result"]["content"][0];
    assert_eq!(image["type"], "image", "{image}");
    assert_eq!(image["mimeType"], "image/png");
    assert_eq!(image["data"], BASE64.encode(PNG));

    assert_eq!(tool_text(&r[2]), ("hello\nworld".to_owned(), false));

    let (text, error) = tool_text(&r[3]);
    assert!(!error, "{text}");
    let summary: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(summary["type"], "application/pdf");
    assert_eq!(summary["size"], PDF.len());
    assert!(
        summary["note"]
            .as_str()
            .unwrap()
            .contains("POLARION_MCP_FILES")
    );

    let (text, error) = tool_text(&r[4]);
    assert!(error && text.contains("POLARION_MCP_FILES"), "{text}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn files_are_exchanged_through_the_folder() {
    let dir = TempDir::new("exchange");
    std::fs::write(dir.0.join("notes.txt"), b"hello").unwrap();
    std::os::unix::fs::symlink(dir.0.join("../outside.pdf"), dir.0.join("link.pdf")).unwrap();
    let server = server().await;
    attachment(&server, "a-pdf", PDF).await;
    mock(
        &server,
        &["createAttachment>", ">notes.txt<", ">aGVsbG8=<"],
        "<createAttachmentResponse/>",
    )
    .await;

    let r = session(
        Some(&server),
        &[
            ("POLARION_MCP_FILES", dir.path()),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
        ],
        &[
            download("a-pdf", json!({"save_as": "spec.pdf"})),
            download("a-pdf", json!({"save_as": "../spec.pdf"})),
            download("a-pdf", json!({"save_as": "link.pdf"})),
            call(
                "polarion_call",
                json!({"command": "items.add-attachment", "args": {
                    "item": "Sandbox/SB-1", "file-name": "notes.txt", "title": "Notes", "data-file": "notes.txt"
                }}),
            ),
        ],
    )
    .await;

    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains(dir.path()), "{instructions}");

    let (text, error) = tool_text(&r[1]);
    assert!(!error, "{text}");
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert!(saved["savedTo"].as_str().unwrap().ends_with("spec.pdf"));
    assert_eq!(saved["size"], PDF.len());
    assert_eq!(std::fs::read(dir.0.join("spec.pdf")).unwrap(), PDF);

    let (text, error) = tool_text(&r[2]);
    assert!(error && text.contains("plain file name"), "{text}");
    let (text, error) = tool_text(&r[3]);
    assert!(error && text.contains("symbolic link"), "{text}");
    assert!(!dir.0.join("../outside.pdf").exists());

    assert_eq!(tool_text(&r[4]), ("done".to_owned(), false));
}

#[tokio::test(flavor = "multi_thread")]
async fn exchange_arguments_are_advertised_only_with_a_folder() {
    let dir = TempDir::new("advertised");
    for with_folder in [false, true] {
        let mut env = vec![("POLARION_MCP_TOOLS", "all")];
        if with_folder {
            env.push(("POLARION_MCP_FILES", dir.path()));
        }
        let r = session(
            None,
            &env,
            &[
                json!({"method": "tools/list"}),
                call(
                    "polarion_describe",
                    json!({"command": "items.add-attachment"}),
                ),
            ],
        )
        .await;
        let tools = r[1]["result"]["tools"].as_array().unwrap();
        let schema =
            |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["inputSchema"].clone();
        let download = schema("items_attachment");
        assert_eq!(download["properties"].get("save_as").is_some(), with_folder);
        let upload = schema("items_add_attachment");
        assert_eq!(upload["properties"].get("data-file").is_some(), with_folder);
        let data_required = upload["required"]
            .as_array()
            .unwrap()
            .contains(&json!("data"));
        assert_eq!(data_required, !with_folder);

        let described: Value = serde_json::from_str(&tool_text(&r[2]).0).unwrap();
        assert_eq!(described["inputSchema"], upload);
    }
}

#[test]
fn a_missing_exchange_folder_stops_the_server() {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion-mcp"));
    for var in VARS {
        cmd.env_remove(var);
    }
    let out = cmd
        .env("POLARION_MCP_FILES", "/nonexistent/polarion-mcp-exchange")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("POLARION_MCP_FILES")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn arguments_are_checked_against_the_command() {
    let r = session(
        None,
        &[],
        &[
            call(
                "polarion_call",
                json!({"command": "items.get", "args": {"item": "P/P-1", "itme": "x"}}),
            ),
            call("query_work_items", json!({"limit": "many"})),
            call("get_work_item", json!({"item": "a/b/c"})),
        ],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(error && text.contains("unknown argument `itme`"), "{text}");
    let (text, error) = tool_text(&r[2]);
    assert!(
        error && text.contains("`limit`: expected an integer"),
        "{text}"
    );
    let (text, error) = tool_text(&r[3]);
    assert!(error && text.contains("`item`"), "{text}");
}

// ---- output schemas

/// `instance` satisfies `schema`.
#[track_caller]
fn conforms(schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}\n{instance}\n{schema}");
}

#[tokio::test(flavor = "multi_thread")]
async fn results_come_with_output_schemas() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>", "projectId>P<", ">P-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id><title>Brakes</title><linkedWorkItems><LinkedWorkItem><role><id>parent</id></role><workItemURI>{URI}P${{WorkItem}}P-0</workItemURI></LinkedWorkItem></linkedWorkItems><type><id>requirement</id></type></getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    mock(
        &server,
        &["getWorkItemsCount>"],
        "<getWorkItemsCountResponse><getWorkItemsCountReturn>7</getWorkItemsCountReturn></getWorkItemsCountResponse>",
    )
    .await;

    let r = session(
        Some(&server),
        &[("POLARION_PROJECT", "P"), ("POLARION_MCP_TOOLS", "all")],
        &[
            json!({"method": "tools/list"}),
            call("get_work_item", json!({"item": "P-1"})),
            call("count_work_items", json!({"query": "type:task"})),
            call("polarion_describe", json!({"command": "items.count"})),
        ],
    )
    .await;

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let output =
        |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["outputSchema"].clone();
    let item_schema = output("get_work_item");
    assert_eq!(item_schema["type"], "object");
    assert!(
        item_schema["properties"]["title"].is_object(),
        "{item_schema}"
    );
    // Not an object: wrapped, so clients of every protocol version accept it.
    let count_schema = output("count_work_items");
    assert_eq!(count_schema["required"], json!(["result"]));
    assert_eq!(count_schema["properties"]["result"]["type"], "integer");
    // Nothing to structure: no result (deletes), file content (attachments).
    assert!(output("items_delete").is_null());
    assert!(output("items_attachment").is_null());
    assert_eq!(
        output("items_query")["properties"]["result"]["type"],
        "array"
    );

    let item = &r[2]["result"];
    let text: Value = serde_json::from_str(item["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(item["structuredContent"], text);
    assert_eq!(item["structuredContent"]["links"][0]["role"], "parent");
    conforms(&item_schema, &item["structuredContent"]);

    let count = &r[3]["result"];
    assert_eq!(count["structuredContent"], json!({"result": 7}));
    conforms(&count_schema, &count["structuredContent"]);

    let described: Value = serde_json::from_str(&tool_text(&r[4]).0).unwrap();
    assert_eq!(described["outputSchema"]["type"], "integer");
}
