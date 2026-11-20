//! `polarion mcp` end to end: JSON-RPC lines on stdin against a mock SOAP server.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SESSION: &str = "/polarion/ws/services/SessionWebService";
const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
const TESTS: &str = "/polarion/ws/services/TestManagementWebService";
const PLANNING: &str = "/polarion/ws/services/PlanningWebService";
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
    "POLARION_WRITE_TARGETS",
    "POLARION_MCP_TOOLS",
    "POLARION_MCP_CONFIG",
    "POLARION_MCP_REQUIRE_GUIDES",
    "POLARION_MCP_FILES",
    "POLARION_MCP_HTTP",
    "POLARION_MCP_AUTH_TOKEN",
    "POLARION_MCP_ALLOW_UNAUTHENTICATED",
    "POLARION_MCP_ALLOWED_ORIGINS",
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
    mock_at(server, TRACKER, needles, body).await;
}

/// A mock of the service at `service` answering one request that contains all `needles`.
async fn mock_at(server: &MockServer, service: &str, needles: &[&str], body: &str) {
    let mut m = Mock::given(path(service));
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
    session_phases(version, server, env, &[requests]).await
}

/// [`session`] with the guides of `guides` read first: their calls are answered before
/// `requests` are sent (rmcp runs the requests of one batch concurrently). The guides'
/// responses are checked and left out, so the result is numbered as for `session`.
async fn guided(
    server: Option<&MockServer>,
    env: &[(&str, &str)],
    guides: &[&str],
    requests: &[Value],
) -> Vec<Value> {
    let calls: Vec<Value> = guides
        .iter()
        .map(|area| call("polarion_guide", json!({"area": area})))
        .collect();
    let mut r = session_phases("2025-06-18", server, env, &[&calls, requests]).await;
    for (response, area) in r[1..=guides.len()].iter().zip(guides) {
        let (text, error) = tool_text(response);
        assert!(!error && text.starts_with("# "), "{area}: {text}");
    }
    r.drain(1..=guides.len());
    r
}

/// A session in phases: the requests of a phase are sent once every response of the phase
/// before is in, and stdin closes after the last phase. Requests are numbered from 1 across the
/// phases; the responses by id, the `initialize` response first.
async fn session_phases(
    version: &str,
    server: Option<&MockServer>,
    env: &[(&str, &str)],
    phases: &[&[Value]],
) -> Vec<Value> {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    cmd.arg("mcp");
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
    // Each phase's input and the ids it waits for.
    let mut batches: Vec<(String, BTreeSet<u64>)> = Vec::new();
    let mut ids = BTreeSet::from([0]);
    let mut next = 1;
    for phase in phases {
        for request in *phase {
            let mut request = request.clone();
            request["jsonrpc"] = json!("2.0");
            request["id"] = json!(next);
            ids.insert(next);
            next += 1;
            input.push_str(&request.to_string());
            input.push('\n');
        }
        batches.push((std::mem::take(&mut input), std::mem::take(&mut ids)));
    }
    let requests = usize::try_from(next - 1).unwrap();
    let (status, lines) = tokio::task::spawn_blocking(move || {
        let mut child = cmd.spawn().expect("binary runs");
        let mut stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            for line in stdout.lines() {
                let _ = tx.send(line.unwrap());
            }
        });
        let mut lines = Vec::new();
        let last = batches.len() - 1;
        for (i, (input, mut pending)) in batches.into_iter().enumerate() {
            stdin.write_all(input.as_bytes()).unwrap();
            if i == last {
                break;
            }
            stdin.flush().unwrap();
            while !pending.is_empty() {
                let line = rx
                    .recv_timeout(Duration::from_secs(120))
                    .expect("a response");
                if let Some(id) = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|v| v["id"].as_u64())
                {
                    pending.remove(&id);
                }
                lines.push(line);
            }
        }
        drop(stdin);
        let status = child.wait().unwrap();
        reader.join().unwrap();
        lines.extend(rx.try_iter());
        (status, lines)
    })
    .await
    .unwrap();
    assert!(status.success());
    let mut responses: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    // One response per request (the notification has none); requests run concurrently, so
    // responses may come in any order.
    responses.sort_by_key(|r| r["id"].as_u64());
    assert_eq!(responses.len(), requests + 1, "{responses:#?}");
    for (i, r) in responses.iter().enumerate() {
        assert_eq!(r["id"], json!(i), "{r}");
    }
    responses
}

fn call(tool: &str, arguments: Value) -> Value {
    json!({"method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

/// The curated read tools, in their order (`CURATED`), by area.
const READ_TOOLS: &[&str] = &[
    "get_work_item",
    "query_work_items",
    "count_work_items",
    "work_item_links",
    "work_item_history",
    "get_work_item_description_markdown",
    "list_documents",
    "get_document",
    "document_work_items",
    "search_documents",
    "export_document_pdf",
    "custom_fields",
    "field_options",
    "search_test_runs",
    "get_test_run",
    "test_case_steps",
    "test_run_records",
    "test_run_report",
    "search_plans",
    "get_plan",
    "plan_statistics",
    "query_wiki_pages",
    "get_wiki_page",
    "get_project",
    "get_user",
];

/// The curated write tools, in their order (`CURATED_WRITES`): listed only with writes enabled.
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

/// The areas of the curated write tools in the instructions, and how many of `WRITE_TOOLS`
/// each has.
const WRITE_AREAS: &[(&str, usize)] = &[
    ("work items", 8),
    ("documents", 3),
    ("test runs", 4),
    ("plans", 4),
];

/// The instructions' sentence naming the curated write tools `listed`, by area.
fn write_tools_sentence(listed: &[&str]) -> String {
    let mut groups = Vec::new();
    let mut tools = WRITE_TOOLS;
    for (area, count) in WRITE_AREAS {
        let (of_area, rest) = tools.split_at(*count);
        tools = rest;
        let names: Vec<String> = of_area
            .iter()
            .filter(|n| listed.contains(n))
            .map(|n| format!("`{n}`"))
            .collect();
        if !names.is_empty() {
            groups.push(format!("{area} ({})", names.join(", ")));
        }
    }
    let groups = match groups.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => unreachable!("some write tools are listed"),
    };
    format!("Write tools of their own for {groups}.")
}

/// The instructions' sentence on the curated write tools that are refused.
fn refused_sentence(refused: &str) -> String {
    format!("Of these, this server refuses {refused}: its write scope does not cover them.")
}

/// The guides and the tools that reach every command, in their order.
const GENERIC_TOOLS: &[&str] = &[
    "polarion_guide",
    "polarion_commands",
    "polarion_describe",
    "polarion_call",
];

/// The names of the tools in a `tools/list` response.
fn tool_names(response: &Value) -> Vec<&str> {
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect()
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
    // Read-only: the Markdown read by its tool, not the writes (by no name).
    assert!(
        instructions.contains("read as Markdown: `get_work_item_description_markdown`.")
            && !instructions.contains("written as Markdown")
            && !instructions.contains("set-description-markdown")
            && !instructions.contains("set_work_item_description_markdown"),
        "{instructions}"
    );

    // Read-only: no write tool of its own either.
    assert!(
        !instructions.contains("tools of their own"),
        "{instructions}"
    );

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let names = tool_names(&r[1]);
    // Read-only: the 25 curated reads and the 4 generic tools, nothing that writes but
    // polarion_call (which the library guards).
    assert_eq!(names.len(), 29, "{names:?}");
    assert_eq!(names, [READ_TOOLS, GENERIC_TOOLS].concat());
    for tool in tools.iter().filter(|t| t["name"] != "polarion_call") {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
    for name in ["get_work_item", "get_work_item_description_markdown"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(tool["inputSchema"]["required"], json!(["item"]), "{name}");
    }

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
    for writes in [false, true] {
        let mut env = vec![("POLARION_MCP_TOOLS", "all")];
        if writes {
            env.push(("POLARION_WRITE_PROJECTS", "Sandbox"));
        }
        let r = session(None, &env, &[json!({"method": "tools/list"})]).await;
        let tools = r[1]["result"]["tools"].as_array().unwrap();
        let names = tool_names(&r[1]);
        // Each command once (the curated ones under their curated names), plus the generic
        // tools; without writes the curated writes are not listed, under no name.
        let commands = polarion_commands::catalog().len();
        let expected = if writes {
            commands
        } else {
            commands - WRITE_TOOLS.len()
        };
        assert_eq!(
            tools.len(),
            GENERIC_TOOLS.len() + expected,
            "writes: {writes}"
        );
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), tools.len(), "a tool listed twice: {names:?}");
        assert!(unique.contains("get_work_item") && !unique.contains("items_get"));
        for generic in [
            "items_create",
            "items_set_description_markdown",
            "items_add_link",
            "documents_query",
            "documents_export_pdf",
            "tests_report",
            "wiki_get",
            "documents_create",
            "tests_update",
            "plans_remove_items",
        ] {
            assert!(!unique.contains(generic), "{generic} listed");
        }
        let curated_writes = if writes { WRITE_TOOLS } else { &[] };
        let first = READ_TOOLS.len() + curated_writes.len();
        assert_eq!(
            names[..first + GENERIC_TOOLS.len()],
            [READ_TOOLS, curated_writes, GENERIC_TOOLS].concat(),
            "writes: {writes}"
        );
        // The other writes keep their generic tools, guarded by the library.
        let delete = tools.iter().find(|t| t["name"] == "items_delete").unwrap();
        assert_eq!(delete["annotations"]["readOnlyHint"], false);
        assert!(
            delete["description"]
                .as_str()
                .unwrap()
                .contains("Modifies data")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn curated_write_tools_are_listed_only_with_writes() {
    let dir = TempDir::new("write-tools");
    let file = dir.0.join("polarion-mcp.toml");
    std::fs::write(
        &file,
        "disable = [\"add_work_item_comment\", \"tests.update\"]",
    )
    .unwrap();
    let disabled = file.to_str().unwrap();
    let listed = [READ_TOOLS, WRITE_TOOLS, GENERIC_TOOLS].concat();
    let unlisted = [READ_TOOLS, GENERIC_TOOLS].concat();
    // A work item and a test run disabled (a new curated write tool): not listed, not named in
    // the instructions, refused by polarion_call.
    let without_disabled: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|n| !["add_work_item_comment", "update_test_run"].contains(n))
        .collect();
    // A work item as the only target: the test run writes but `set_test_steps` (a test case is a
    // work item), and the document and plan writes, are refused; so are creates.
    let item_only = refused_sentence(
        "the document and plan write tools, `create_work_item`, `create_test_run`, \
         `update_test_run` and `add_test_record`",
    );
    let global_only = refused_sentence("the work item, document, test run and plan write tools");
    // Targets of every kind: only the creates are refused.
    let plan = format!("{URI}Sandbox${{Plan}}rel1");
    let all_kinds = format!("SB-1, doc:Specs/SRS, run:RUN-1, {plan}");
    let creates_only = refused_sentence(
        "`create_work_item`, `create_document`, `create_test_run` and `create_plan`",
    );
    // A test run as the only target, and of its area only `set_test_steps` listed: the tool is
    // named, not the test run writes (they would go through).
    let steps_only = refused_sentence("`set_test_steps`");
    // The environment, the tools it lists, and the refusal the instructions state.
    type Case<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str], Option<&'a str>);
    let cases: [Case; 11] = [
        (&[("POLARION_PROJECT", "Sandbox")], &unlisted, None),
        (&[("POLARION_WRITE_PROJECTS", "Sandbox")], &listed, None),
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "SB-1"),
            ],
            &listed,
            Some(&item_only),
        ),
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", &all_kinds),
            ],
            &listed,
            Some(&creates_only),
        ),
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "run:RUN-1"),
                ("POLARION_MCP_TOOLS", "set_test_steps, get_work_item"),
            ],
            &[
                "get_work_item",
                "set_test_steps",
                "polarion_guide",
                "polarion_commands",
                "polarion_describe",
                "polarion_call",
            ],
            Some(&steps_only),
        ),
        (
            &[("POLARION_ALLOW_GLOBAL_WRITES", "1")],
            &listed,
            Some(&global_only),
        ),
        // Global writes next to write projects add nothing to refuse.
        (
            &[
                ("POLARION_ALLOW_GLOBAL_WRITES", "1"),
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ],
            &listed,
            None,
        ),
        // Empty targets: read-only, whatever else is set.
        (
            &[
                ("POLARION_WRITE_TARGETS", ""),
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ],
            &unlisted,
            None,
        ),
        (
            &[
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
                ("POLARION_MCP_CONFIG", disabled),
            ],
            &without_disabled,
            None,
        ),
        // Named by tool or command name: listed only with writes.
        (
            &[
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
                (
                    "POLARION_MCP_TOOLS",
                    "items.update, create_work_item, tests.update, get_plan",
                ),
            ],
            &[
                "get_plan",
                "create_work_item",
                "update_work_item",
                "update_test_run",
                "polarion_guide",
                "polarion_commands",
                "polarion_describe",
                "polarion_call",
            ],
            None,
        ),
        (
            &[(
                "POLARION_MCP_TOOLS",
                "items.update, create_work_item, tests.update, get_plan",
            )],
            &[
                "get_plan",
                "polarion_guide",
                "polarion_commands",
                "polarion_describe",
                "polarion_call",
            ],
            None,
        ),
    ];
    for (env, expected, refusal) in cases {
        let r = session(
            None,
            env,
            &[
                json!({"method": "tools/list"}),
                call(
                    "polarion_call",
                    json!({"command": "items.add-comment", "args": {"item": "Sandbox/SB-1", "text": "x"}}),
                ),
                call(
                    "polarion_call",
                    json!({"command": "tests.update", "args": {"run": format!("{URI}Sandbox${{TestRun}}RUN-1"), "title": "x"}}),
                ),
            ],
        )
        .await;
        let names = tool_names(&r[1]);
        assert_eq!(names, expected, "{env:?}");
        let write_tools: Vec<&str> = WRITE_TOOLS
            .iter()
            .copied()
            .filter(|n| names.contains(n))
            .collect();
        let instructions = r[0]["result"]["instructions"].as_str().unwrap();
        if write_tools.is_empty() {
            assert!(
                !instructions.contains("tools of their own"),
                "{instructions}"
            );
        } else {
            let mention = write_tools_sentence(&write_tools);
            assert!(instructions.contains(&mention), "{env:?}: {instructions}");
        }
        // What the scope cannot reach is listed, but the instructions say it is refused
        // (`the_write_tools_are_refused_where_the_scope_cannot_reach`).
        match refusal {
            Some(refusal) => assert!(instructions.contains(refusal), "{env:?}: {instructions}"),
            None => assert!(
                !instructions.contains("this server refuses"),
                "{env:?}: {instructions}"
            ),
        }
        // A disabled command is refused by polarion_call too. With writes enabled, the others
        // need their area's guide first; without, they fail here for want of a server (the
        // library's refusal without writes is `writes_are_refused_without_write_projects`).
        let disabled = env.iter().any(|(k, _)| *k == "POLARION_MCP_CONFIG");
        for (response, area) in r[2..].iter().zip(["work-items", "tests"]) {
            let (text, error) = tool_text(response);
            assert!(error, "{text}");
            assert_eq!(text.contains("is disabled"), disabled, "{env:?}: {text}");
            if disabled {
                continue;
            }
            if write_tools.is_empty() {
                assert!(text.starts_with("error (config):"), "{env:?}: {text}");
            } else {
                assert!(
                    text.starts_with("error (guideRequired):")
                        && text.contains(&format!("{{\"area\": \"{area}\"}}")),
                    "{env:?}: {text}"
                );
            }
        }
    }

    // The write tools: marked as writes, with the command's own schemas.
    let r = session(
        None,
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &[
            json!({"method": "tools/list"}),
            call(
                "polarion_describe",
                json!({"command": "items.set-description-markdown"}),
            ),
        ],
    )
    .await;
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    for name in WRITE_TOOLS {
        let tool = tools.iter().find(|t| t["name"] == *name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{name}");
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("Modifies data"),
            "{name}: {description}"
        );
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
    }
    let described: Value = serde_json::from_str(&tool_text(&r[2]).0).unwrap();
    let markdown = tools
        .iter()
        .find(|t| t["name"] == "set_work_item_description_markdown")
        .unwrap();
    assert_eq!(markdown["inputSchema"], described["inputSchema"]);
    assert_eq!(markdown["description"], described["description"]);
    assert_eq!(markdown["outputSchema"], described["outputSchema"]);
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
    // The commands of the curated writes too: no tool of their own (unknown tool names), but
    // polarion_call reaches them, and the library refuses them.
    let item = "P/P-1";
    let commands = [
        ("items.delete", json!({"item": item})),
        (
            "items.create",
            json!({"project": "P", "type": "task", "title": "New"}),
        ),
        ("items.update", json!({"item": item, "title": "New"})),
        (
            "items.set-description-markdown",
            json!({"item": item, "markdown": "# Brakes"}),
        ),
        (
            "items.append-description-markdown",
            json!({"item": item, "markdown": "More."}),
        ),
        ("items.add-comment", json!({"item": item, "text": "x"})),
        (
            "items.add-link",
            json!({"item": item, "role": "relates_to", "target": "P/P-2"}),
        ),
        (
            "items.remove-link",
            json!({"item": item, "role": "relates_to", "target": "P/P-2"}),
        ),
        (
            "items.add-attachment",
            json!({"item": item, "file-name": "notes.txt", "title": "Notes", "data": "aGVsbG8="}),
        ),
    ];
    let mut requests: Vec<Value> = commands
        .iter()
        .map(|(command, args)| call("polarion_call", json!({"command": command, "args": args})))
        .collect();
    requests.push(call(
        "create_work_item",
        json!({"type": "task", "title": "New"}),
    ));
    let r = session(Some(&server), &[], &requests).await;
    for (response, (command, _)) in r[1..].iter().zip(&commands) {
        let (text, error) = tool_text(response);
        assert!(
            error && text.starts_with("error (readOnly):"),
            "{command}: {text}"
        );
    }
    assert_eq!(r[commands.len() + 1]["error"]["code"], -32602, "{r:?}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

/// Markdown and the description HTML it converts to (and back from), escaped as XML text.
const MARKDOWN: &str = "### Brakes\n\nMust stop the car.\n";
const HTML: &str = "&lt;span style=&quot;font-weight: bold; font-size: 14pt; line-height: 1.2;&quot;&gt;Brakes&lt;/span&gt;&lt;br/&gt;&lt;br/&gt;Must stop the car.";

#[tokio::test(flavor = "multi_thread")]
async fn descriptions_are_read_as_markdown_text() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemByIdsWithFields>", "workitemId>P-1<"],
        &format!(
            r#"<getWorkItemByIdsWithFieldsResponse><getWorkItemByIdsWithFieldsReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id><description><type>text/html</type><content>{HTML}</content></description></getWorkItemByIdsWithFieldsReturn></getWorkItemByIdsWithFieldsResponse>"#
        ),
    )
    .await;

    let r = session(
        Some(&server),
        &[("POLARION_PROJECT", "P")],
        &[
            json!({"method": "tools/list"}),
            call("get_work_item_description_markdown", json!({"item": "P-1"})),
        ],
    )
    .await;

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|t| t["name"] == "get_work_item_description_markdown")
        .unwrap();
    // A string: wrapped, so clients of every protocol version accept it.
    let schema = &tool["outputSchema"];
    assert_eq!(schema["properties"]["result"]["type"], "string", "{schema}");

    // The Markdown itself as text (not JSON), and as structured content.
    let result = &r[2]["result"];
    let (text, error) = tool_text(&r[2]);
    assert!(!error, "{text}");
    assert_eq!(result["content"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result["content"][0]["type"], "text");
    assert_eq!(text.trim_end_matches('\n'), MARKDOWN.trim_end_matches('\n'));
    assert_eq!(result["structuredContent"], json!({"result": text}));
    conforms(schema, &result["structuredContent"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn descriptions_are_written_through_the_markdown_tool() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemByIdsWithFields>", "workitemId>SB-1<"],
        &format!(
            r#"<getWorkItemByIdsWithFieldsResponse><getWorkItemByIdsWithFieldsReturn uri="{URI}Sandbox${{WorkItem}}SB-1"><id>SB-1</id></getWorkItemByIdsWithFieldsReturn></getWorkItemByIdsWithFieldsResponse>"#
        ),
    )
    .await;
    mock(
        &server,
        &[
            "updateWorkItem>",
            &format!("{URI}Sandbox${{WorkItem}}SB-1"),
            "Must stop the car.",
        ],
        "<updateWorkItemResponse/>",
    )
    .await;
    let r = guided(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
        ],
        &["work-items"],
        &[
            json!({"method": "tools/list"}),
            call(
                "set_work_item_description_markdown",
                json!({"item": "SB-1", "markdown": MARKDOWN}),
            ),
        ],
    )
    .await;
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let schema = &tools
        .iter()
        .find(|t| t["name"] == "set_work_item_description_markdown")
        .unwrap()["outputSchema"];
    let result = &r[2]["result"];
    let (text, error) = tool_text(&r[2]);
    assert!(!error, "{text}");
    let update: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(update["updated"], true, "{update}");
    assert_eq!(result["structuredContent"], update);
    conforms(schema, &update);
    let requests = tracker_requests(&server).await;
    assert_eq!(requests.len(), 2, "{requests:#?}");
    assert!(requests[1].contains("updateWorkItem>"), "{}", requests[1]);

    // Restricted to SB-1: another item, the target's id in another project and a create are
    // refused through the write tools before anything is sent (by the library: the gate is off).
    let untouched = MockServer::start().await;
    let r = session(
        Some(&untouched),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1"),
            ("POLARION_MCP_REQUIRE_GUIDES", "none"),
        ],
        &[
            json!({"method": "tools/list"}),
            call(
                "set_work_item_description_markdown",
                json!({"item": "SB-2", "markdown": MARKDOWN}),
            ),
            call(
                "append_work_item_description_markdown",
                json!({"item": "Other/SB-1", "markdown": "More."}),
            ),
            call("create_work_item", json!({"type": "task", "title": "New"})),
        ],
    )
    .await;
    assert!(tool_names(&r[1]).contains(&"set_work_item_description_markdown"));
    for response in &r[2..] {
        let (text, error) = tool_text(response);
        assert!(error && text.starts_with("error (readOnly):"), "{text}");
    }
    assert!(
        untouched.received_requests().await.unwrap().is_empty(),
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
    let r = guided(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &["work-items"],
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

// ---- write targets

/// The bodies of the requests the tracker service received, but the guides' reads.
async fn tracker_requests(server: &MockServer) -> Vec<String> {
    requests_to(server, &[TRACKER]).await
}

/// The bodies of the requests the `services` received, but the reads of `polarion_guide`.
async fn requests_to(server: &MockServer, services: &[&str]) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| services.contains(&r.url.path()))
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|body| !GUIDE_READS.iter().any(|op| body.contains(op)))
        .collect()
}

/// The operations `polarion_guide` reads the project section with.
const GUIDE_READS: &[&str] = &[
    "getAllEnumOptionsForKey>",
    "getAllEnumOptionsForId>",
    "getDefinedCustomFieldKeys>",
];

#[tokio::test(flavor = "multi_thread")]
async fn writes_go_only_to_write_targets() {
    let server = server().await;
    mock(
        &server,
        &[
            "addAssignee>",
            &format!("{URI}Sandbox${{WorkItem}}SB-1<"),
            "assigneeId>jdoe<",
        ],
        "<addAssigneeResponse><addAssigneeReturn>true</addAssigneeReturn></addAssigneeResponse>",
    )
    .await;
    let r = guided(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1, doc:Specs/SRS, run:RUN-1"),
            // No effect next to the targets.
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ("POLARION_ALLOW_GLOBAL_WRITES", "1"),
        ],
        &["work-items"],
        &[
            call(
                "polarion_call",
                json!({"command": "items.add-assignee", "args": {"item": "SB-1", "user": "jdoe"}}),
            ),
            // Another item of the same project, a write with no bounds, a create.
            call(
                "polarion_call",
                json!({"command": "items.add-assignee", "args": {"item": "Sandbox/SB-2", "user": "jdoe"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "items.delete", "args": {"item": "SB-1"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "items.create", "args": {"type": "task", "title": "New"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "items.set-description-markdown", "args": {"item": "SB-2", "markdown": "# Brakes"}}),
            ),
            // The targets were resolved once, in POLARION_PROJECT: the target's id in another
            // project is another item, and a create's `project` changes nothing either.
            call(
                "polarion_call",
                json!({"command": "items.add-assignee", "args": {"item": "Other/SB-1", "user": "jdoe"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "items.create", "args": {"project": "Sandbox", "type": "task", "title": "New"}}),
            ),
        ],
    )
    .await;
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(&format!(
            "Writes are restricted to these objects: work item Sandbox/SB-1, document \
             Sandbox/Specs/SRS, test run {URI}Sandbox${{TestRun}}RUN-1. Every other write is \
             refused, and every object a write names (link targets, test cases, defects, plan \
             items) must be one of them too."
        )),
        "{instructions}"
    );
    assert!(
        instructions.contains(
            "A change can still mark links from other work items to a target \
                               as suspect"
        ),
        "{instructions}"
    );
    assert!(
        !instructions.contains("Writes are enabled"),
        "{instructions}"
    );
    // The target is a work item: its description can be written as Markdown, by the write
    // tools that are listed with writes enabled.
    assert!(
        instructions.contains(
            "read and written as Markdown: `get_work_item_description_markdown`, \
             `set_work_item_description_markdown`, `append_work_item_description_markdown`."
        ),
        "{instructions}"
    );

    assert_eq!(tool_text(&r[1]), (r#"{"changed":true}"#.to_owned(), false));
    for response in &r[2..] {
        let (text, error) = tool_text(response);
        assert!(error && text.starts_with("error (readOnly):"), "{text}");
    }
    // The write to the target is the only request to the tracker.
    let requests = tracker_requests(&server).await;
    assert_eq!(requests.len(), 1, "{requests:#?}");
    assert!(requests[0].contains("addAssignee>"), "{}", requests[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_write_targets_refuse_every_write() {
    // Empty, or only commas and blanks.
    for targets in ["", "  ,  , "] {
        let untouched = MockServer::start().await;
        let r = session(
        Some(&untouched),
        &[
            ("POLARION_WRITE_TARGETS", targets),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ("POLARION_ALLOW_GLOBAL_WRITES", "1"),
        ],
        &[
            call(
                "polarion_call",
                json!({"command": "items.add-assignee", "args": {"item": "Sandbox/SB-1", "user": "jdoe"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "items.set-description-markdown", "args": {"item": "Sandbox/SB-1", "markdown": "# Brakes"}}),
            ),
            call(
                "polarion_call",
                json!({"command": "users.create", "args": {"login": "jdoe"}}),
            ),
        ],
    )
    .await;
        let instructions = r[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("This server is read-only.")
                && !instructions.contains("written as Markdown")
                && !instructions.contains("tools of their own"),
            "{instructions}"
        );
        for response in &r[1..] {
            let (text, error) = tool_text(response);
            assert!(error && text.starts_with("error (readOnly):"), "{text}");
        }
        assert!(
            untouched.received_requests().await.unwrap().is_empty(),
            "nothing may be sent, not even a login"
        );
    }
}

/// Writes are enabled, but the scope cannot reach some areas (one kind of object as the only
/// target, or global writes alone): the curated write tools are listed, the instructions say
/// which are refused and offer the Markdown read only, and the library refuses them before
/// anything is sent.
#[tokio::test(flavor = "multi_thread")]
async fn the_write_tools_are_refused_where_the_scope_cannot_reach() {
    let run = format!("{URI}Sandbox${{TestRun}}RUN-1");
    let plan = format!("{URI}Sandbox${{Plan}}rel1");
    // The environment, the write scope the instructions state, and the refused write tools.
    type Case<'a> = (&'a [(&'a str, &'a str)], String, &'a str);
    let cases: [Case; 4] = [
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "doc:Specs/SRS"),
            ],
            "Writes are restricted to these objects: document Sandbox/Specs/SRS.".to_owned(),
            "the work item, test run and plan write tools and `create_document`",
        ),
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_ALLOW_GLOBAL_WRITES", "1"),
            ],
            "Writes are enabled for: global writes.".to_owned(),
            "the work item, document, test run and plan write tools",
        ),
        // A test run: its writes may go through, but not `set_test_steps` (a test case is a
        // work item), `add_test_record` (it names a test case) and the create.
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "run:RUN-1"),
            ],
            format!("Writes are restricted to these objects: test run {run}."),
            "the work item, document and plan write tools, `create_test_run`, `add_test_record` and `set_test_steps`",
        ),
        // A plan, by its URI: its updates may go through, but not the create, and not the plan
        // item changes (plan items are work items).
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", &plan),
            ],
            format!("Writes are restricted to these objects: {plan}."),
            "the work item, document and test run write tools, `create_plan`, `add_plan_items` and `remove_plan_items`",
        ),
    ];
    for (env, scope, refused) in cases {
        let untouched = MockServer::start().await;
        // The library's refusals: the gate is off.
        let env = [env, &[("POLARION_MCP_REQUIRE_GUIDES", "none")]].concat();
        let env = &env[..];
        let r = session(
            Some(&untouched),
            env,
            &[
                json!({"method": "tools/list"}),
                call(
                    "set_work_item_description_markdown",
                    json!({"item": "SB-1", "markdown": "# Brakes"}),
                ),
                call("create_work_item", json!({"type": "task", "title": "New"})),
                call(
                    "add_work_item_comment",
                    json!({"item": "SB-1", "text": "x"}),
                ),
                // The new write tools, on objects that are not targets.
                call(
                    "create_document",
                    json!({"name": "New", "allowed-types": ["requirement"]}),
                ),
                call(
                    "add_document_comment",
                    json!({"document": "Specs/Other", "text": "x"}),
                ),
                call(
                    "update_test_run",
                    json!({"run": format!("{URI}Sandbox${{TestRun}}RUN-2"), "title": "x"}),
                ),
                call(
                    "set_test_steps",
                    json!({"test-case": "SB-1", "steps": [["Brake"]]}),
                ),
                call(
                    "add_plan_items",
                    json!({"plan": format!("{URI}Sandbox${{Plan}}rel2"), "items": ["SB-1"]}),
                ),
                call(
                    "create_plan",
                    json!({"id": "rel3", "name": "Release 3", "template": "release"}),
                ),
            ],
        )
        .await;
        let instructions = r[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(&scope)
                && instructions.contains("read as Markdown: `get_work_item_description_markdown`.")
                && !instructions.contains("written as Markdown")
                && instructions.contains(&write_tools_sentence(WRITE_TOOLS))
                && instructions.contains(&refused_sentence(refused)),
            "{env:?}: {instructions}"
        );
        assert_eq!(
            tool_names(&r[1]),
            [READ_TOOLS, WRITE_TOOLS, GENERIC_TOOLS].concat(),
            "{env:?}"
        );
        for response in &r[2..] {
            let (text, error) = tool_text(response);
            assert!(
                error && text.starts_with("error (readOnly):"),
                "{env:?}: {text}"
            );
        }
        assert!(
            untouched.received_requests().await.unwrap().is_empty(),
            "{env:?}: nothing may be sent, not even a login"
        );
    }
}

/// The new curated tools reach Polarion: a read (`test_run_report`), and with a test run as the
/// only target a write to it (`update_test_run`); a write to another run is refused before
/// anything is sent.
#[tokio::test(flavor = "multi_thread")]
async fn test_run_tools_read_and_write_the_target() {
    let run = format!("{URI}Sandbox${{TestRun}}RUN-1");
    let server = server().await;
    mock_at(
        &server,
        TESTS,
        &["getWikiContentForTestRun>", &format!("testRunUri>{run}<")],
        "<getWikiContentForTestRunResponse><getWikiContentForTestRunReturn><type>text/html</type><content>All passed.</content></getWikiContentForTestRunReturn></getWikiContentForTestRunResponse>",
    )
    .await;
    mock_at(
        &server,
        TESTS,
        &["updateTestRun>", &format!("uri=\"{run}\""), ">Night run<"],
        "<updateTestRunResponse/>",
    )
    .await;
    let r = guided(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "run:RUN-1"),
        ],
        &["tests"],
        &[
            json!({"method": "tools/list"}),
            call("test_run_report", json!({"run": run})),
            call("update_test_run", json!({"run": run, "title": "Night run"})),
            call(
                "update_test_run",
                json!({"run": format!("{URI}Sandbox${{TestRun}}RUN-2"), "title": "Night run"}),
            ),
        ],
    )
    .await;
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(&refused_sentence(
            "the work item, document and plan write tools, `create_test_run`, `add_test_record` and `set_test_steps`"
        )),
        "{instructions}"
    );

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let tool = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    assert_eq!(tool("test_run_report")["annotations"]["readOnlyHint"], true);
    assert_eq!(
        tool("update_test_run")["annotations"]["readOnlyHint"],
        false
    );
    let schema = &tool("test_run_report")["outputSchema"];

    let (text, error) = tool_text(&r[2]);
    assert!(!error, "{text}");
    let report = json!({"type": "text/html", "content": "All passed."});
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), report);
    assert_eq!(r[2]["result"]["structuredContent"], report);
    conforms(schema, &report);

    assert_eq!(tool_text(&r[3]), ("done".to_owned(), false));
    let (text, error) = tool_text(&r[4]);
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    // The report and the update to the target are the only test management requests.
    let requests: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == TESTS)
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert_eq!(requests.len(), 2, "{requests:#?}");
}

/// A plan and a document as the targets: the new curated write tools reach them
/// (`update_plan`, `add_document_comment`); the same tools on other plans and documents, and plan
/// items that are not targets, are refused before anything is sent.
#[tokio::test(flavor = "multi_thread")]
async fn plan_and_document_tools_write_their_targets() {
    let plan = format!("{URI}Sandbox${{Plan}}rel1");
    let document = format!("{URI}Sandbox${{Module}}{{moduleFolder}}Specs#SRS");
    let server = server().await;
    mock_at(
        &server,
        PLANNING,
        &["updatePlan>", &format!("{plan}\""), "name>Release 1.1<"],
        "<updatePlanResponse/>",
    )
    .await;
    mock(
        &server,
        &["addComment>", &format!("{document}<"), "Looks good"],
        &format!(
            "<addCommentResponse><addCommentReturn>{document}%23c1</addCommentReturn></addCommentResponse>"
        ),
    )
    .await;
    let targets = format!("doc:Specs/SRS, {plan}");
    let r = guided(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", &targets),
        ],
        &["plans", "documents"],
        &[
            call("update_plan", json!({"plan": plan, "name": "Release 1.1"})),
            call(
                "add_document_comment",
                json!({"document": "Specs/SRS", "text": "Looks good"}),
            ),
            call(
                "update_plan",
                json!({"plan": format!("{URI}Sandbox${{Plan}}rel2"), "name": "x"}),
            ),
            call(
                "add_document_comment",
                json!({"document": "Specs/Other", "text": "x"}),
            ),
            call("add_plan_items", json!({"plan": plan, "items": ["SB-1"]})),
        ],
    )
    .await;
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(&refused_sentence(
            "the work item and test run write tools, `create_document`, `create_plan`, `add_plan_items` and `remove_plan_items`"
        )),
        "{instructions}"
    );
    assert_eq!(tool_text(&r[1]), ("done".to_owned(), false));
    let (text, error) = tool_text(&r[2]);
    assert!(!error && text.contains("Specs#SRS"), "{text}");
    for response in &r[3..] {
        let (text, error) = tool_text(response);
        assert!(error && text.starts_with("error (readOnly):"), "{text}");
    }
    // The two writes to the targets are the only planning and tracker requests (but the
    // guides' reads).
    let requests = requests_to(&server, &[PLANNING, TRACKER]).await;
    assert_eq!(requests.len(), 2, "{requests:#?}");
}

#[test]
fn write_targets_that_do_not_resolve_stop_the_server() {
    for (targets, project, message) in [
        (
            "SB-1, doc:Specs",
            "Sandbox",
            "invalid write target `doc:Specs`",
        ),
        ("SB-1", "", "write target `SB-1` has no project"),
    ] {
        let stderr = refused(
            &[],
            &[
                ("POLARION_WRITE_TARGETS", targets),
                ("POLARION_PROJECT", project),
            ],
        );
        assert!(stderr.contains(message), "{stderr}");
    }
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

    let r = guided(
        Some(&server),
        &[
            ("POLARION_MCP_FILES", dir.path()),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
        ],
        &["work-items"],
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

/// `export_document_pdf` returns file content: a size-and-type summary without an exchange
/// folder, saved with `save_as` with one; no output schema either way.
#[tokio::test(flavor = "multi_thread")]
async fn document_pdf_exports_are_byte_results() {
    let dir = TempDir::new("pdf-export");
    for with_folder in [false, true] {
        let server = server().await;
        mock(
            &server,
            &[
                "exportDocumentToPDF>",
                &format!("{URI}P${{Module}}{{moduleFolder}}Specs#SRS<"),
            ],
            &format!(
                "<exportDocumentToPDFResponse><exportDocumentToPDFReturn>{}</exportDocumentToPDFReturn></exportDocumentToPDFResponse>",
                BASE64.encode(PDF)
            ),
        )
        .await;
        let mut env = vec![("POLARION_PROJECT", "P")];
        let mut args = json!({"document": "Specs/SRS"});
        if with_folder {
            env.push(("POLARION_MCP_FILES", dir.path()));
            args["save_as"] = json!("srs.pdf");
        }
        let r = session(
            Some(&server),
            &env,
            &[
                json!({"method": "tools/list"}),
                call("export_document_pdf", args),
            ],
        )
        .await;
        let tools = r[1]["result"]["tools"].as_array().unwrap();
        let tool = tools
            .iter()
            .find(|t| t["name"] == "export_document_pdf")
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(tool.get("outputSchema").is_none(), "{tool}");
        assert_eq!(
            tool["inputSchema"]["properties"].get("save_as").is_some(),
            with_folder
        );

        let result = &r[2]["result"];
        assert!(result.get("structuredContent").is_none(), "{result}");
        let (text, error) = tool_text(&r[2]);
        assert!(!error, "{text}");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["size"], PDF.len());
        if with_folder {
            assert!(value["savedTo"].as_str().unwrap().ends_with("srs.pdf"));
            assert_eq!(std::fs::read(dir.0.join("srs.pdf")).unwrap(), PDF);
        } else {
            assert_eq!(value["type"], "application/pdf");
            assert!(
                value["note"]
                    .as_str()
                    .unwrap()
                    .contains("POLARION_MCP_FILES")
            );
            assert!(!text.contains(&BASE64.encode(PDF)), "{text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn exchange_arguments_are_advertised_only_with_a_folder() {
    let dir = TempDir::new("advertised");
    for with_folder in [false, true] {
        // With writes enabled: the upload has a curated tool, listed only then.
        let mut env = vec![
            ("POLARION_MCP_TOOLS", "all"),
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
        ];
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
        let upload = schema("add_work_item_attachment");
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
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    cmd.arg("mcp");
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

// ---- tool selection

/// The instructions name the Markdown commands only as far as they are offered.
#[tokio::test(flavor = "multi_thread")]
async fn the_instructions_name_only_the_markdown_commands_offered() {
    let instructions = |env: &[(&str, &str)], config: Option<&str>| {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let config = config.map(str::to_owned);
        async move {
            let dir = TempDir::new("instructions");
            let file = dir.0.join("polarion-mcp.toml");
            let mut env: Vec<(&str, &str)> =
                env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            if let Some(config) = &config {
                std::fs::write(&file, config).unwrap();
                env.push(("POLARION_MCP_CONFIG", file.to_str().unwrap()));
            }
            let r = session(None, &env, &[]).await;
            r[0]["result"]["instructions"].as_str().unwrap().to_owned()
        }
    };

    let writes = instructions(&[("POLARION_WRITE_PROJECTS", "P")], None).await;
    assert!(
        writes.contains(
            "read and written as Markdown: `get_work_item_description_markdown`, \
             `set_work_item_description_markdown`, `append_work_item_description_markdown`. \
             Mermaid code blocks"
        ),
        "{writes}"
    );

    // Not selected: through polarion_call.
    let called = instructions(
        &[
            ("POLARION_WRITE_PROJECTS", "P"),
            ("POLARION_MCP_TOOLS", "items.get"),
        ],
        None,
    )
    .await;
    assert!(
        called.contains(
            "read and written as Markdown: `items.description-markdown` (through \
             `polarion_call`), `items.set-description-markdown` (through `polarion_call`), \
             `items.append-description-markdown` (through `polarion_call`). Mermaid code blocks"
        ),
        "{called}"
    );

    let tools = instructions(
        &[
            ("POLARION_WRITE_PROJECTS", "P"),
            ("POLARION_MCP_TOOLS", "items.set-description-markdown"),
        ],
        Some("disable = [\"polarion_call\"]"),
    )
    .await;
    assert!(
        tools.contains("can be written as Markdown: `set_work_item_description_markdown`. Mermaid"),
        "{tools}"
    );

    for (env, config) in [
        (
            &[("POLARION_WRITE_PROJECTS", "P")][..],
            "disable = [\"items.description-markdown\", \"items.set-description-markdown\", \
             \"items.append-description-markdown\"]",
        ),
        (
            &[][..],
            "tools = [\"items.get\"]\ndisable = [\"polarion_call\"]",
        ),
    ] {
        let none = instructions(env, Some(config)).await;
        assert!(!none.contains("Markdown"), "{config}: {none}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_config_file_selects_and_disables_tools() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>", "projectId>P<", ">P-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id><title>Brakes</title></getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    let dir = TempDir::new("config");
    let file = dir.0.join("polarion-mcp.toml");
    std::fs::write(
        &file,
        r#"
        # Work items only, no deletes, no describe tool, no output schemas
        tools = ["items.*"]
        disable = ["items.delete", "polarion_describe"]
        output_schemas = false
        "#,
    )
    .unwrap();
    let r = session(
        Some(&server),
        &[
            ("POLARION_PROJECT", "P"),
            ("POLARION_MCP_CONFIG", file.to_str().unwrap()),
        ],
        &[
            json!({"method": "tools/list"}),
            call("get_work_item", json!({"item": "P-1"})),
            call(
                "polarion_call",
                json!({"command": "items.delete", "args": {"items": ["P-1"]}}),
            ),
            call("polarion_commands", json!({"area": "items"})),
            call("polarion_commands", json!({})),
        ],
    )
    .await;

    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("Some commands are disabled")
            && !instructions.contains("polarion_describe"),
        "{instructions}"
    );
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for name in [
        "get_work_item",
        "items_add_hyperlink",
        "polarion_call",
        "polarion_commands",
    ] {
        assert!(names.contains(&name), "{name} missing: {names:?}");
    }
    // Read-only: the curated writes are not listed, under no name.
    for name in [
        "items_delete",
        "polarion_describe",
        "list_documents",
        "items_get",
        "add_work_item_link",
        "items_add_link",
    ] {
        assert!(!names.contains(&name), "{name} listed: {names:?}");
    }
    assert!(tools.iter().all(|t| t.get("outputSchema").is_none()));

    let item = &r[2]["result"];
    assert!(tool_text(&r[2]).0.contains("Brakes"));
    assert!(item.get("structuredContent").is_none(), "{item}");

    let (text, error) = tool_text(&r[3]);
    assert!(
        error && text.contains("`items.delete` is disabled"),
        "{text}"
    );

    let (text, _) = tool_text(&r[4]);
    let listed: Value = serde_json::from_str(&text).unwrap();
    let listed: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"items.get") && !listed.contains(&"items.delete"));

    let (text, _) = tool_text(&r[5]);
    let areas: Value = serde_json::from_str(&text).unwrap();
    let items = areas
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["area"] == "items")
        .unwrap();
    assert_eq!(items["commands"], listed.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tools_variable_takes_selectors_and_wins_over_the_file() {
    let dir = TempDir::new("config-env");
    let file = dir.0.join("polarion-mcp.toml");
    std::fs::write(&file, "tools = \"all\"").unwrap();
    let r = session(
        None,
        &[
            ("POLARION_MCP_CONFIG", file.to_str().unwrap()),
            ("POLARION_MCP_TOOLS", "documents.*, get_project"),
        ],
        &[json!({"method": "tools/list"})],
    )
    .await;
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names[..5],
        [
            "list_documents",
            "get_document",
            "document_work_items",
            "search_documents",
            "export_document_pdf"
        ]
    );
    assert!(names.contains(&"get_project") && names.contains(&"documents_delete"));
    // A curated write: listed only with writes enabled, under no name.
    assert!(
        !names.contains(&"documents_create") && !names.contains(&"create_document"),
        "{names:?}"
    );
    assert!(!names.contains(&"get_work_item"), "{names:?}");
}

/// Start the server with `args` and `env`, expect it to stop with exit code 1; its stderr.
#[track_caller]
fn refused(args: &[String], env: &[(&str, &str)]) -> String {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    cmd.arg("mcp");
    for var in VARS {
        cmd.env_remove(var);
    }
    let out = cmd
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{args:?} {env:?}");
    String::from_utf8(out.stderr).unwrap()
}

#[test]
fn a_bad_config_stops_the_server() {
    let dir = TempDir::new("config-bad");
    let typo = dir.0.join("typo.toml");
    std::fs::write(&typo, "tools = [\"itmes.*\"]").unwrap();
    let unknown_key = dir.0.join("key.toml");
    std::fs::write(&unknown_key, "tool = \"all\"").unwrap();
    let missing = dir.0.join("missing.toml");

    let stderr = refused(&["--config".into(), typo.display().to_string()], &[]);
    assert!(stderr.contains("`itmes.*` matches no command"), "{stderr}");
    let stderr = refused(&[format!("--config={}", unknown_key.display())], &[]);
    assert!(stderr.contains("unknown field `tool`"), "{stderr}");
    let stderr = refused(&[], &[("POLARION_MCP_CONFIG", missing.to_str().unwrap())]);
    assert!(stderr.contains("missing.toml"), "{stderr}");
    let stderr = refused(&[], &[("POLARION_MCP_TOOLS", "itmes.*")]);
    assert!(stderr.contains("POLARION_MCP_TOOLS: `itmes.*`"), "{stderr}");
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
        output("query_work_items")["properties"]["result"]["type"],
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

/// Rendering a large diagram keeps one thread busy for a while; other requests are answered
/// meanwhile, whether the write comes through its curated tool or through `polarion_call`.
#[tokio::test(flavor = "multi_thread")]
async fn requests_are_answered_while_a_diagram_renders() {
    for curated in [true, false] {
        answered_while_rendering(curated).await;
    }
}

/// [`requests_are_answered_while_a_diagram_renders`], the write through
/// `set_work_item_description_markdown` (listed with writes enabled) or `polarion_call`.
async fn answered_while_rendering(curated: bool) {
    use std::time::Instant;

    let server = server().await;
    mock(
        &server,
        &["getWorkItemByIdsWithFields>"],
        &format!(
            r#"<getWorkItemByIdsWithFieldsResponse><getWorkItemByIdsWithFieldsReturn uri="{URI}P${{WorkItem}}P-1"><id>P-1</id></getWorkItemByIdsWithFieldsReturn></getWorkItemByIdsWithFieldsResponse>"#
        ),
    )
    .await;
    // A diagram close to the renderer's edge limit. The upload is not mocked: the call fails
    // after rendering, which is all this test needs.
    let mut diagram = "graph TD\n".to_owned();
    for i in 0..299 {
        diagram += &format!("    N{} --> N{}\n", i % 97, (i * 7 + 13) % 97 + 97);
    }
    let markdown = format!("```mermaid\n{diagram}```\n");

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    cmd.arg("mcp");
    for var in VARS {
        cmd.env_remove(var);
    }
    let cache = TempDir::new("render-cache");
    cmd.env("POLARION_URL", format!("{}/polarion", server.uri()))
        .env("POLARION_TOKEN", "t")
        .env("POLARION_WRITE_PROJECTS", "P")
        // About rendering: the gate is off.
        .env("POLARION_MCP_REQUIRE_GUIDES", "none")
        .env("XDG_CACHE_HOME", cache.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().expect("binary runs");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<(Instant, Value)>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            let _ = tx.send((Instant::now(), serde_json::from_str(&line).unwrap()));
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, message: Value| {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    };
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}
        }}),
    );
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    let args = json!({"item": "P/P-1", "markdown": markdown});
    let mut write = if curated {
        call("set_work_item_description_markdown", args)
    } else {
        call(
            "polarion_call",
            json!({"command": "items.set-description-markdown", "args": args}),
        )
    };
    write["jsonrpc"] = json!("2.0");
    write["id"] = json!(1);
    send(&mut stdin, write);

    // Once the description is read, the diagram renders.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let requests = server.received_requests().await.unwrap();
        if requests
            .iter()
            .any(|r| String::from_utf8_lossy(&r.body).contains("getWorkItemByIdsWithFields>"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "the description was not read");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pinged = Instant::now();
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
    );
    let mut answered = std::collections::BTreeMap::new();
    while answered.len() < 3 {
        let (at, response) = rx.recv_timeout(Duration::from_secs(120)).unwrap();
        answered.insert(response["id"].as_u64().unwrap(), (at, response));
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();

    let (written, response) = &answered[&1];
    assert_eq!(
        response["result"]["isError"], true,
        "curated: {curated}: {response}"
    );
    let (ping, _) = &answered[&2];
    let (ping, write) = (
        ping.saturating_duration_since(pinged),
        written.saturating_duration_since(pinged),
    );
    assert!(
        ping * 4 < write,
        "curated: {curated}: the ping took {ping:?}, the write {write:?} after it"
    );
}

// ---- guides

/// The guides' Markdown, by area, in their order.
const GUIDES: &[(&str, &str)] = &[
    (
        "overview",
        include_str!("../../polarion-mcp/guides/overview.md"),
    ),
    (
        "queries",
        include_str!("../../polarion-mcp/guides/queries.md"),
    ),
    (
        "work-items",
        include_str!("../../polarion-mcp/guides/work-items.md"),
    ),
    (
        "documents",
        include_str!("../../polarion-mcp/guides/documents.md"),
    ),
    ("tests", include_str!("../../polarion-mcp/guides/tests.md")),
    ("plans", include_str!("../../polarion-mcp/guides/plans.md")),
    ("wiki", include_str!("../../polarion-mcp/guides/wiki.md")),
];

/// One enum option as the tracker returns it.
fn enum_option(element: &str, id: &str, name: &str, hidden: bool) -> String {
    format!(
        "<{element}><default>false</default><enumId>e</enumId><hidden>{hidden}</hidden><id>{id}</id><name>{name}</name><phantom>false</phantom><sequenceNumber>0</sequenceNumber></{element}>"
    )
}

/// Answers for the reads of `polarion_guide` in project `P`, any number of times: the types
/// (`requirement`, `task`), with `all` also the statuses (`draft`, `open`, a hidden `obsolete`),
/// the link roles (`parent`, `relates_to`) and the custom field keys of each type.
async fn guide_reads(server: &MockServer, all: bool) {
    let options = |op: &str, values: &[(&str, &str, bool)]| {
        let element = format!("{op}Return");
        let items: String = values
            .iter()
            .map(|(id, name, hidden)| enum_option(&element, id, name, *hidden))
            .collect();
        format!("<{op}Response>{items}</{op}Response>")
    };
    let keys = |keys: &[&str]| {
        let items: String = keys
            .iter()
            .map(|k| {
                format!("<getDefinedCustomFieldKeysReturn>{k}</getDefinedCustomFieldKeysReturn>")
            })
            .collect();
        format!("<getDefinedCustomFieldKeysResponse>{items}</getDefinedCustomFieldKeysResponse>")
    };
    let mut answers = vec![(
        ["getAllEnumOptionsForKey>", "projectID>P<", "key>type<"],
        options(
            "getAllEnumOptionsForKey",
            &[
                ("requirement", "Requirement", false),
                ("task", "Task", false),
            ],
        ),
    )];
    if all {
        answers.extend([
            (
                ["getAllEnumOptionsForKey>", "projectID>P<", "key>status<"],
                options(
                    "getAllEnumOptionsForKey",
                    &[
                        ("draft", "Draft", false),
                        ("obsolete", "Obsolete", true),
                        ("open", "Open", false),
                    ],
                ),
            ),
            (
                [
                    "getAllEnumOptionsForId>",
                    "projectID>P<",
                    "enumID>workitem-link-role<",
                ],
                options(
                    "getAllEnumOptionsForId",
                    &[
                        ("parent", "Parent", false),
                        ("relates_to", "Relates to", false),
                    ],
                ),
            ),
            (
                [
                    "getDefinedCustomFieldKeys>",
                    "projectID>P<",
                    "typeID>requirement<",
                ],
                keys(&["asil", "reviewed"]),
            ),
            (
                ["getDefinedCustomFieldKeys>", "projectID>P<", "typeID>task<"],
                keys(&[]),
            ),
        ]);
    }
    for (needles, body) in answers {
        let mut m = Mock::given(path(TRACKER));
        for n in needles {
            m = m.and(body_string_contains(n));
        }
        m.respond_with(xml(&body)).mount(server).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guide_tool_returns_each_area_with_the_project_section() {
    let server = server().await;
    guide_reads(&server, true).await;
    let mut requests = vec![json!({"method": "tools/list"})];
    requests.extend(
        GUIDES
            .iter()
            .map(|(area, _)| call("polarion_guide", json!({"area": area}))),
    );
    requests.extend([
        // A project the mock knows nothing about: its reads fail.
        call(
            "polarion_guide",
            json!({"area": "work-items", "project": "Other"}),
        ),
        call("polarion_guide", json!({"area": "items"})),
        call("polarion_guide", json!({"area": "wiki", "projekt": "P"})),
        call("polarion_guide", json!({})),
    ]);
    let r = session(Some(&server), &[("POLARION_PROJECT", "P")], &requests).await;

    let tools = r[1]["result"]["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|t| t["name"] == "polarion_guide")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    let areas: Vec<&str> = GUIDES.iter().map(|(area, _)| *area).collect();
    assert_eq!(
        tool["inputSchema"]["properties"]["area"]["enum"],
        json!(areas)
    );
    assert_eq!(tool["inputSchema"]["required"], json!(["area"]));
    assert!(tool.get("outputSchema").is_none(), "{tool}");
    let description = tool["description"].as_str().unwrap();
    assert!(
        description.contains("`work-items`: Work items"),
        "{description}"
    );

    for (i, (area, guide)) in GUIDES.iter().enumerate() {
        let (text, error) = tool_text(&r[2 + i]);
        assert!(!error, "{area}: {text}");
        let guide = guide.trim_end();
        assert!(text.starts_with(guide), "{area}: {text}");
        let section = &text[guide.len()..];
        assert!(
            section.starts_with("\n\n## Project `P` (read from the server)\n\n"),
            "{area}: {section}"
        );
        assert!(
            section.contains("- Work item types: `requirement` (Requirement), `task` (Task)\n"),
            "{area}: {section}"
        );
        // The hidden status is left out.
        assert!(
            section.contains("- Statuses (`status`): `draft` (Draft), `open` (Open)\n"),
            "{area}: {section}"
        );
        let roles =
            section.contains("- Link roles: `parent` (Parent), `relates_to` (Relates to)\n");
        assert_eq!(
            roles,
            matches!(*area, "work-items" | "queries"),
            "{area}: {section}"
        );
        let custom = section.contains(
            "- Custom field keys by type:\n  - `requirement`: `asil`, `reviewed`\n  - `task`: none\n",
        );
        assert_eq!(custom, *area == "work-items", "{area}: {section}");
    }

    // Reads that fail give a note, not an error.
    let (text, error) = tool_text(&r[9]);
    assert!(!error, "{text}");
    assert!(
        text.contains(
            "\n\n## Project `Other` (read from the server)\n\nThe project's configuration could \
             not be read: "
        ) && !text.contains("- Work item types"),
        "{text}"
    );
    for (response, message) in r[10..].iter().zip([
        "unknown area `items`: one of overview, queries, work-items, documents, tests, plans, wiki",
        "unknown argument `projekt`",
        "unknown area ``",
    ]) {
        let (text, error) = tool_text(response);
        assert!(
            error && text.starts_with("error (invalid):") && text.contains(message),
            "{text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_project_section_notes_what_it_cannot_read() {
    // No default project and no `project`.
    let r = session(
        None,
        &[],
        &[call("polarion_guide", json!({"area": "queries"}))],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(
        !error
            && text.ends_with(
                "\n\n## Project\n\nNo default project: call `polarion_guide` with `project` to \
                 list a project's work item types, statuses and link roles here.\n"
            ),
        "{text}"
    );

    // No server configured.
    let r = session(
        None,
        &[("POLARION_PROJECT", "P")],
        &[call("polarion_guide", json!({"area": "tests"}))],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(
        !error
            && text.contains(
                "## Project `P` (read from the server)\n\nThe project's configuration could not \
                 be read: "
            ),
        "{text}"
    );

    // The types are read, the rest fails: one note per line.
    let server = server().await;
    guide_reads(&server, false).await;
    let r = session(
        Some(&server),
        &[],
        &[call(
            "polarion_guide",
            json!({"area": "work-items", "project": "P"}),
        )],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(!error, "{text}");
    for line in [
        "- Work item types: `requirement` (Requirement), `task` (Task)\n",
        "- Statuses (`status`): could not be read (",
        "- Link roles: could not be read (",
        "  - `requirement`: could not be read (",
        "  - `task`: could not be read (",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guides_are_resources() {
    let r = session(
        None,
        &[],
        &[
            json!({"method": "resources/list"}),
            json!({"method": "resources/read", "params": {"uri": "polarion://guide/queries"}}),
            json!({"method": "resources/read", "params": {"uri": "polarion://guide/items"}}),
            json!({"method": "resources/templates/list"}),
        ],
    )
    .await;
    assert!(
        r[0]["result"]["capabilities"]["resources"].is_object(),
        "{}",
        r[0]
    );
    assert!(r[0]["result"]["capabilities"]["tools"].is_object());
    let resources = r[1]["result"]["resources"].as_array().unwrap();
    assert_eq!(resources.len(), GUIDES.len());
    for (resource, (area, text)) in resources.iter().zip(GUIDES) {
        assert_eq!(resource["uri"], format!("polarion://guide/{area}"));
        assert_eq!(resource["name"], *area);
        assert_eq!(resource["mimeType"], "text/markdown");
        assert_eq!(resource["size"], text.len());
        assert!(
            resource["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty())
        );
    }
    // The static text, without the project section.
    assert_eq!(
        r[2]["result"]["contents"],
        json!([{"uri": "polarion://guide/queries", "mimeType": "text/markdown", "text": GUIDES[1].1}])
    );
    assert_eq!(r[3]["error"]["code"], -32002, "{}", r[3]);
    assert_eq!(r[4]["result"]["resourceTemplates"], json!([]));
}

/// The text of a refusal by the gate: the command must wait for `area`'s guide.
#[track_caller]
fn assert_needs_guide(response: &Value, area: &str) {
    let (text, error) = tool_text(response);
    assert!(
        error
            && text.starts_with("error (guideRequired):")
            && text.contains(&format!(
                "call polarion_guide with {{\"area\": \"{area}\"}}"
            ))
            && text.contains("nothing was sent"),
        "{area}: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_need_their_areas_guide_first() {
    // Before the guide: the writes of every area are refused by the gate, curated tools and
    // polarion_call alike, and nothing is sent, not even a login. Reading a guide as a resource
    // does not count.
    let untouched = MockServer::start().await;
    let env = [
        ("POLARION_PROJECT", "Sandbox"),
        ("POLARION_WRITE_PROJECTS", "Sandbox"),
    ];
    let writes = [
        (
            call("update_work_item", json!({"item": "SB-1", "title": "x"})),
            "work-items",
        ),
        (
            call(
                "polarion_call",
                json!({"command": "items.add-assignee", "args": {"item": "SB-1", "user": "jdoe"}}),
            ),
            "work-items",
        ),
        (
            call(
                "add_document_comment",
                json!({"document": "Specs/SRS", "text": "x"}),
            ),
            "documents",
        ),
        (
            call(
                "update_test_run",
                json!({"run": format!("{URI}Sandbox${{TestRun}}RUN-1"), "title": "x"}),
            ),
            "tests",
        ),
        (
            call(
                "update_plan",
                json!({"plan": format!("{URI}Sandbox${{Plan}}rel1"), "name": "x"}),
            ),
            "plans",
        ),
        (
            call(
                "polarion_call",
                json!({"command": "users.create", "args": {"login": "jdoe"}}),
            ),
            "overview",
        ),
    ];
    let calls: Vec<Value> = writes.iter().map(|(c, _)| c.clone()).collect();
    let r = session_phases(
        "2025-06-18",
        Some(&untouched),
        &env,
        &[
            &[json!({"method": "resources/read", "params": {"uri": "polarion://guide/work-items"}})],
            &calls,
        ],
    )
    .await;
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.starts_with(
            "Access to a Siemens Polarion ALM server. Read `polarion_guide(area)` before using \
             an area's tools (`overview`, `queries`, `work-items`, `documents`, `tests`, `plans`, \
             `wiki`): the writes of an area are refused until its guide was read. Work item \
             commands need `work-items`; documents, tests, plans and wiki their own guide; every \
             other area `overview`. "
        ),
        "{instructions}"
    );
    assert!(r[1]["result"]["contents"].is_array(), "{}", r[1]);
    for (response, (_, area)) in r[2..].iter().zip(&writes) {
        assert_needs_guide(response, area);
    }
    assert!(
        untouched.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );

    // Reads are not gated; after polarion_guide the area's writes go through (as far as the
    // library allows them), the other areas' still wait for their guides.
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>", ">SB-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}Sandbox${{WorkItem}}SB-1"><id>SB-1</id><title>Brakes</title></getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    mock(
        &server,
        &["addAssignee>", "assigneeId>jdoe<"],
        "<addAssigneeResponse><addAssigneeReturn>true</addAssigneeReturn></addAssigneeResponse>",
    )
    .await;
    let assign = call(
        "polarion_call",
        json!({"command": "items.add-assignee", "args": {"item": "SB-1", "user": "jdoe"}}),
    );
    let r = session_phases(
        "2025-06-18",
        Some(&server),
        &env,
        &[
            &[
                call("get_work_item", json!({"item": "SB-1"})),
                assign.clone(),
            ],
            &[call("polarion_guide", json!({"area": "work-items"}))],
            &[assign, writes[3].0.clone()],
        ],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(!error && text.contains("Brakes"), "{text}");
    assert_needs_guide(&r[2], "work-items");
    let (text, error) = tool_text(&r[3]);
    assert!(!error && text.starts_with("# Work items"), "{text}");
    assert_eq!(tool_text(&r[4]), (r#"{"changed":true}"#.to_owned(), false));
    assert_needs_guide(&r[5], "tests");
    // The mock answers `addAssignee` once: the refused call was not sent.
    let assigned = tracker_requests(&server)
        .await
        .iter()
        .filter(|b| b.contains("addAssignee>"))
        .count();
    assert_eq!(assigned, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn all_gates_every_command_and_none_no_command() {
    let dir = TempDir::new("require-guides");
    let file = dir.0.join("all.toml");
    std::fs::write(&file, "require_guides = \"all\"").unwrap();
    for env in [
        ("POLARION_MCP_REQUIRE_GUIDES", "all"),
        ("POLARION_MCP_CONFIG", file.to_str().unwrap()),
    ] {
        let untouched = MockServer::start().await;
        let r = session(
            Some(&untouched),
            &[("POLARION_PROJECT", "P"), env],
            &[
                call("get_work_item", json!({"item": "P-1"})),
                call("custom_fields", json!({"type": "requirement"})),
                call("polarion_call", json!({"command": "wiki.spaces"})),
                call("polarion_commands", json!({"area": "items"})),
                call("polarion_describe", json!({"command": "items.get"})),
            ],
        )
        .await;
        let instructions = r[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("every command of an area is refused until its guide was read."),
            "{env:?}: {instructions}"
        );
        assert_needs_guide(&r[1], "work-items");
        assert_needs_guide(&r[2], "overview");
        assert_needs_guide(&r[3], "wiki");
        for response in &r[4..] {
            let (text, error) = tool_text(response);
            assert!(!error, "{env:?}: {text}");
        }
        assert!(untouched.received_requests().await.unwrap().is_empty());
    }

    // `none`: the write goes to the library, which decides (a target elsewhere: refused).
    let untouched = MockServer::start().await;
    let r = session(
        Some(&untouched),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1"),
            ("POLARION_MCP_REQUIRE_GUIDES", "none"),
        ],
        &[call(
            "update_work_item",
            json!({"item": "SB-2", "title": "x"}),
        )],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    assert!(untouched.received_requests().await.unwrap().is_empty());
    // Read-only with the default: nothing can be written, so nothing waits for a guide.
    let untouched = MockServer::start().await;
    let read_only = session(
        Some(&untouched),
        &[],
        &[call(
            "polarion_call",
            json!({"command": "items.delete", "args": {"item": "P/P-1"}}),
        )],
    )
    .await;
    let (text, error) = tool_text(&read_only[1]);
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    assert!(untouched.received_requests().await.unwrap().is_empty());
    for r in [&r, &read_only] {
        let instructions = r[0]["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(
                "`polarion_guide(area)` explains each area of this server (`overview`, `queries`, \
                 `work-items`, `documents`, `tests`, `plans`, `wiki`) and lists the project's work \
                 item types and statuses; read it before working in an area you do not know."
            ) && !instructions.contains("until its guide was read"),
            "{instructions}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_guide_tool_no_guide_is_required() {
    let dir = TempDir::new("no-guide");
    let file = dir.0.join("polarion-mcp.toml");
    std::fs::write(&file, "disable = [\"polarion_guide\"]").unwrap();
    let file = file.to_str().unwrap();
    let untouched = MockServer::start().await;
    let r = session(
        Some(&untouched),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1"),
            ("POLARION_MCP_CONFIG", file),
        ],
        &[
            json!({"method": "tools/list"}),
            call("polarion_guide", json!({"area": "overview"})),
            call("update_work_item", json!({"item": "SB-2", "title": "x"})),
            json!({"method": "resources/list"}),
        ],
    )
    .await;
    let names = tool_names(&r[1]);
    assert_eq!(
        names,
        [READ_TOOLS, WRITE_TOOLS, &GENERIC_TOOLS[1..]].concat()
    );
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(!instructions.contains("polarion_guide"), "{instructions}");
    assert_eq!(r[2]["error"]["code"], -32602);
    // The library's refusal, not the gate's.
    let (text, error) = tool_text(&r[3]);
    assert!(error && text.starts_with("error (readOnly):"), "{text}");
    // The resources stay.
    assert_eq!(
        r[4]["result"]["resources"].as_array().unwrap().len(),
        GUIDES.len()
    );
    assert!(untouched.received_requests().await.unwrap().is_empty());

    // Asking for a gate without the tool, or for an unknown one, stops the server.
    let stderr = refused(
        &[],
        &[
            ("POLARION_MCP_CONFIG", file),
            ("POLARION_MCP_REQUIRE_GUIDES", "write"),
        ],
    );
    assert!(
        stderr.contains("POLARION_MCP_REQUIRE_GUIDES needs `polarion_guide`, which is disabled"),
        "{stderr}"
    );
    let stderr = refused(&[], &[("POLARION_MCP_REQUIRE_GUIDES", "sometimes")]);
    assert!(
        stderr.contains("POLARION_MCP_REQUIRE_GUIDES: expected `write`, `all` or `none`"),
        "{stderr}"
    );
    let typo = dir.0.join("typo.toml");
    std::fs::write(&typo, "require_guides = \"reads\"").unwrap();
    let stderr = refused(&["--config".into(), typo.display().to_string()], &[]);
    assert!(stderr.contains("unknown variant `reads`"), "{stderr}");
}

// ---- Markdown files with images

/// `Sandbox/<id>` as `getWorkItemByIdsWithFields` returns it.
fn described(id: &str, description: &str, attachments: &[(&str, &str)]) -> String {
    let mut fields = format!("<id>{id}</id>");
    if !description.is_empty() {
        fields += &format!(
            "<description><type>text/html</type><content>{}</content></description>",
            description
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        );
    }
    if !attachments.is_empty() {
        fields += "<attachments>";
        for (attachment, file_name) in attachments {
            fields += &format!(
                "<Attachment><fileName>{file_name}</fileName><id>{attachment}</id></Attachment>"
            );
        }
        fields += "</attachments>";
    }
    format!(
        r#"<getWorkItemByIdsWithFieldsResponse><getWorkItemByIdsWithFieldsReturn uri="{URI}Sandbox${{WorkItem}}{id}">{fields}</getWorkItemByIdsWithFieldsReturn></getWorkItemByIdsWithFieldsResponse>"#
    )
}

/// Answer the `n`-th read of `Sandbox/<id>` (counting from 1) with `body`.
async fn read_nth(server: &MockServer, id: &str, n: u8, body: &str) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByIdsWithFields>"))
        .and(body_string_contains(format!("workitemId>{id}<")))
        .respond_with(xml(body))
        .up_to_n_times(1)
        .with_priority(n)
        .expect(1)
        .mount(server)
        .await;
}

fn set_from_file(file: &str) -> Value {
    call(
        "set_work_item_description_markdown",
        json!({"item": "Sandbox/SB-1", "file": file}),
    )
}

fn export_to(dir: &str) -> Value {
    call(
        "polarion_call",
        json!({"command": "items.export-description", "args": {"item": "Sandbox/SB-2", "dir": dir}}),
    )
}

/// Commands that take a path work inside the exchange folder, and only there.
#[tokio::test(flavor = "multi_thread")]
async fn markdown_files_and_exports_stay_in_the_exchange_folder() {
    let dir = TempDir::new("markdown-files");
    std::fs::create_dir_all(dir.0.join("img")).unwrap();
    std::fs::write(dir.0.join("img/a.png"), PNG).unwrap();
    std::fs::write(dir.0.join("doc.md"), "Shot: ![s](img/a.png)\n").unwrap();
    let env = [
        ("POLARION_MCP_FILES", dir.path()),
        ("POLARION_WRITE_PROJECTS", "Sandbox"),
    ];

    let server = server().await;
    // The write: the image is attached, and the description shows the attachment.
    read_nth(&server, "SB-1", 1, &described("SB-1", "", &[])).await;
    read_nth(
        &server,
        "SB-1",
        2,
        &described("SB-1", "", &[("1-a.png", "a.png")]),
    )
    .await;
    mock(
        &server,
        &["createAttachment>", "fileName>a.png<", &BASE64.encode(PNG)],
        "<createAttachmentResponse/>",
    )
    .await;
    mock(
        &server,
        &["updateWorkItem>", "workitemimg:1-a.png"],
        "<updateWorkItemResponse/>",
    )
    .await;
    // The export: the image the description shows is saved next to the Markdown.
    read_nth(
        &server,
        "SB-2",
        1,
        &described(
            "SB-2",
            r#"Text <img src="workitemimg:1-b.png" style="max-width: 650px;"/>"#,
            &[("1-b.png", "b.png")],
        ),
    )
    .await;
    attachment(&server, "1-b.png", PNG).await;

    let r = guided(
        Some(&server),
        &env,
        &["work-items"],
        &[
            set_from_file("doc.md"),
            export_to("out"),
            // Outside the folder: refused before anything is asked of the server.
            set_from_file("../doc.md"),
            export_to("../out"),
        ],
    )
    .await;

    let (text, error) = tool_text(&r[1]);
    assert!(!error, "{text}");
    let update: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        update,
        json!({"updated": true, "images": [{"url": "img/a.png", "attachment": "1-a.png", "uploaded": true}]})
    );

    let (text, error) = tool_text(&r[2]);
    assert!(!error, "{text}");
    let export: Value = serde_json::from_str(&text).unwrap();
    assert!(
        export["markdown"].as_str().unwrap().ends_with("SB-2.md"),
        "{export}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.0.join("out/SB-2.md")).unwrap(),
        "Text ![](SB-2/1-b.png)"
    );
    assert_eq!(std::fs::read(dir.0.join("out/SB-2/1-b.png")).unwrap(), PNG);

    for response in &r[3..=4] {
        let (text, error) = tool_text(response);
        assert!(error && text.contains("outside the folder"), "{text}");
    }
    assert!(!dir.0.join("../out").exists());
}

/// Without an exchange folder the server reads and writes no file.
#[tokio::test(flavor = "multi_thread")]
async fn paths_are_refused_without_an_exchange_folder() {
    let server = MockServer::start().await;
    let r = guided(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &["work-items"],
        &[set_from_file("doc.md")],
    )
    .await;
    let (text, error) = tool_text(&r[1]);
    assert!(
        error && text.contains("local files are not available"),
        "{text}"
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}
