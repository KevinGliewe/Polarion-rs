//! The front-end tier of the live suite for the `polarion-mcp` server over stdio, against a real
//! server (see docs/testing.md and `crates/polarion/tests/live.rs`).
//!
//! ```sh
//! cargo test -p polarion-mcp --test live -- --ignored --nocapture
//! ```
//!
//! The environment of the write tier (`POLARION_URL`, `POLARION_TOKEN`, `POLARION_PROJECT`,
//! `POLARION_LIVE_WRITES`, `POLARION_LIVE_ITEMS`, `POLARION_LIVE_TEST_RUN`,
//! `POLARION_LIVE_DOCUMENT`, `POLARION_LIVE_USER`). After the gates of the write tier, the server
//! runs with `POLARION_WRITE_TARGETS` set to A's URI and every other `POLARION_*` variable
//! removed (the MCP config and tools included), one session per step: `tools/list` (the curated
//! read and write tools are listed in order, and the instructions name the Markdown writes and
//! refuse the write tools of the areas A is not in), `get_work_item_description_markdown` on A,
//! `test_run_report` on R and `test_case_steps` on B (the library's results, as JSON);
//! `polarion_guide` for work items (with the project section), then
//! `set_work_item_description_markdown` writing A's description (restored afterwards through the
//! library); and the canary through `polarion_call`: refused by the guide gate before the guide
//! was read, then, after `polarion_guide`, refused by the library with `readOnly`. Then the
//! fixtures are compared with the baseline, and the audit runs.

#[macro_use]
#[path = "../../polarion/tests/live_support.rs"]
mod live_support;

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::Stdio;
use std::time::Duration;

use live_support::*;
use polarion::{Fields, WorkItemUpdate};
use serde_json::{Value, json};

/// One stdio session at protocol 2025-06-18: `initialize`, then `requests` (numbered from 1);
/// the responses by id, the `initialize` response first.
async fn session(g: &Gate, requests: &[Value]) -> Result<Vec<Value>, String> {
    session_phases(g, &[requests]).await
}

/// A session in phases: a phase's requests are sent once every response of the phase before is
/// in (the server runs the requests of one batch concurrently, so a guide read in the same batch
/// as a write may come too late for it). Requests are numbered from 1 across the phases.
async fn session_phases(g: &Gate, phases: &[&[Value]]) -> Result<Vec<Value>, String> {
    let mut cmd = front_end(
        env!("CARGO_BIN_EXE_polarion-mcp"),
        &g.settings,
        &g.fixtures.a.uri,
    );
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut input = String::new();
    input.push_str(
        &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "polarion-rs live test", "version": "0"}
        }})
        .to_string(),
    );
    input.push('\n');
    input.push_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    input.push('\n');
    let mut batches: Vec<(String, BTreeSet<u64>)> = Vec::new();
    let mut ids = BTreeSet::from([0]);
    let mut next = 1u64;
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
    let (status, lines) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot run polarion-mcp: {e}"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let mut lines = Vec::new();
        let last = batches.len().saturating_sub(1);
        let mut failed = None;
        for (i, (input, mut pending)) in batches.into_iter().enumerate() {
            if let Err(e) = stdin
                .write_all(input.as_bytes())
                .and_then(|()| stdin.flush())
            {
                failed = Some(format!("cannot write to polarion-mcp: {e}"));
                break;
            }
            if i == last {
                break;
            }
            while !pending.is_empty() {
                let Ok(line) = rx.recv_timeout(Duration::from_secs(300)) else {
                    failed = Some(format!("no responses for {pending:?}"));
                    break;
                };
                if let Some(id) = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|v| v["id"].as_u64())
                {
                    pending.remove(&id);
                }
                lines.push(line);
            }
            if failed.is_some() {
                break;
            }
        }
        // The server sees the end of its input.
        drop(stdin);
        let status = child.wait().map_err(|e| e.to_string())?;
        let _ = reader.join();
        lines.extend(rx.try_iter());
        match failed {
            Some(e) => Err(e),
            None => Ok((status, lines)),
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    if !status.success() {
        return Err(format!("polarion-mcp exited with {:?}", status.code()));
    }
    let mut responses = Vec::new();
    for line in &lines {
        let response: Value =
            serde_json::from_str(line).map_err(|e| format!("not JSON ({e}): {}", cut(line)))?;
        responses.push(response);
    }
    responses.sort_by_key(|r| r["id"].as_u64());
    let ids: Vec<Option<u64>> = responses.iter().map(|r| r["id"].as_u64()).collect();
    let expected: Vec<Option<u64>> = (0..next).map(Some).collect();
    if ids != expected {
        return Err(format!("responses {ids:?}, expected {expected:?}"));
    }
    Ok(responses)
}

fn call(tool: &str, arguments: Value) -> Value {
    json!({"method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

/// The curated read tools, in their order.
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

/// The curated write tools, in their order: listed because writes are enabled (the target).
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

/// The text of a tool result, and whether it is an error.
fn tool_text(response: &Value) -> (String, bool) {
    let result = &response["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: writes to fixture A on the server in POLARION_URL (see docs/testing.md)"]
async fn front_end_mcp() {
    let _tier = one_tier("front end (MCP)");
    let mut report = Report::default();
    if let Some(g) = write_gate(&mut report).await {
        let result = steps(&mut report, &g).await;
        if g.after(&mut report, "MCP", result).await {
            if index_probe(&mut report, &g).await {
                report.ok("the index shows the run's last change");
            }
            g.after(&mut report, "index probe", Ok(())).await;
        } else {
            wait_for_index(&mut report, &g).await;
        }
        audit(&mut report, &g, false).await;
        g.close(&mut report).await;
    }
    report.finish("front end (MCP)");
}

async fn steps(report: &mut Report, g: &Gate) -> Result<(), String> {
    let f = &g.fixtures;

    let r = session(
        g,
        &[
            json!({"method": "tools/list"}),
            call(
                "get_work_item_description_markdown",
                json!({"item": f.a.uri}),
            ),
            call("test_run_report", json!({"run": f.r})),
            call("test_case_steps", json!({"test-case": f.b.uri})),
        ],
    )
    .await?;
    let instructions = r[0]["result"]["instructions"].as_str().unwrap_or_default();
    // A work item is the only target: the document, test run (but `set_test_steps`, which
    // writes a test case) and plan write tools are refused, and so are creates.
    check!(
        report,
        instructions.contains("Writes are restricted to these objects")
            && instructions.contains("read and written as Markdown")
            && instructions.contains("`set_work_item_description_markdown`")
            && instructions.contains(
                "Of these, this server refuses the document and plan write tools, \
                 `create_work_item`, `create_test_run`, `update_test_run` and \
                 `add_test_record`: its write scope does not cover them."
            ),
        "MCP: the instructions name the write target, the Markdown writes and the refused write \
         tools",
        "instructions: {}",
        cut(instructions)
    );
    let names: Vec<&str> = r[1]["result"]["tools"]
        .as_array()
        .map(|tools| tools.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    check!(
        report,
        ["polarion_call", "get_work_item_description_markdown"]
            .iter()
            .all(|n| names.contains(n)),
        "MCP: tools/list offers polarion_call and the Markdown read",
        "tools {names:?}"
    );
    let curated = [READ_TOOLS, WRITE_TOOLS].concat();
    check!(
        report,
        names.get(..curated.len()) == Some(&curated[..]),
        "MCP: tools/list offers the 25 curated read and 19 curated write tools first, in order",
        "tools {names:?}"
    );
    let (text, error) = tool_text(&r[2]);
    check!(
        report,
        !error && text.contains("polarion-rs live test fixture A"),
        "MCP: get_work_item_description_markdown on A",
        "error: {error}, text {}",
        cut(&text)
    );
    let tests = g.reader.tests();
    let expected = tests
        .report(&f.r)
        .await
        .map_err(|e| format!("R's report: {e}"))
        .map(|t| serde_json::to_value(t).expect("results serialize"))?;
    let (text, error) = tool_text(&r[3]);
    check!(
        report,
        !error && serde_json::from_str::<Value>(&text).ok() == Some(expected),
        "MCP: test_run_report on R is the library's report",
        "error: {error}, text {}",
        cut(&text)
    );
    let expected = tests
        .steps(&f.b)
        .await
        .map_err(|e| format!("B's test steps: {e}"))
        .map(|s| serde_json::to_value(s).expect("results serialize"))?;
    let (text, error) = tool_text(&r[4]);
    check!(
        report,
        !error && serde_json::from_str::<Value>(&text).ok() == Some(expected),
        "MCP: test_case_steps on B are the library's steps",
        "error: {error}, text {}",
        cut(&text)
    );

    let before = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    let token = run_token();
    let markdown = format!("polarion-rs live test of the MCP server ({token}).");
    // The guide first: the server refuses work item writes until it was read.
    let r = session_phases(
        g,
        &[
            &[call("polarion_guide", json!({"area": "work-items"}))],
            &[call(
                "set_work_item_description_markdown",
                json!({"item": f.a.uri, "markdown": markdown}),
            )],
        ],
    )
    .await?;
    let (text, error) = tool_text(&r[1]);
    check!(
        report,
        !error
            && text.starts_with("# Work items")
            && text.contains(&format!("## Project `{}`", g.settings.project)),
        "MCP: polarion_guide for work items, with the project section",
        "error: {error}, text {}",
        cut(&text)
    );
    let (text, error) = tool_text(&r[2]);
    check!(
        report,
        !error,
        "MCP: set_work_item_description_markdown on A",
        "{}",
        cut(&text)
    );
    let written = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    check!(
        report,
        written.as_ref().is_some_and(|d| d.content.contains(&token)),
        "MCP: A's description is the HTML of the Markdown",
        "description {:?}",
        written.as_ref().map(|d| cut(&d.content))
    );
    let items = g.writer.items();
    let restored = match &before {
        Some(text) => {
            items
                .update(
                    &f.a,
                    &WorkItemUpdate {
                        description: Some(text.clone()),
                        ..WorkItemUpdate::default()
                    },
                )
                .await
        }
        None => items.clear_fields(&f.a, &["description".to_owned()]).await,
    };
    restored.map_err(|e| format!("restoring A's description: {e}"))?;
    let after = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    check!(
        report,
        after == before,
        "MCP: A's description restored",
        "description {:?}",
        after.as_ref().map(|d| cut(&d.content))
    );

    // The canary: before the guide the gate refuses it (nothing is sent); after the guide the
    // library does, since the canary is no write target.
    let canary = call(
        "polarion_call",
        json!({"command": "items.remove-hyperlink", "args": {"item": g.canary.uri, "url": CANARY_URL}}),
    );
    let r = session_phases(
        g,
        &[
            std::slice::from_ref(&canary),
            &[call("polarion_guide", json!({"area": "work-items"}))],
            std::slice::from_ref(&canary),
        ],
    )
    .await?;
    let (text, error) = tool_text(&r[1]);
    check!(
        report,
        error && text.starts_with("error (guideRequired):") && text.contains("\"work-items\""),
        "MCP: polarion_call items.remove-hyperlink on the canary before the guide is refused by \
         the gate (guideRequired)",
        "error: {error}, text {}",
        cut(&text)
    );
    let (text, error) = tool_text(&r[3]);
    check!(
        report,
        error && text.starts_with("error (readOnly):"),
        "MCP: polarion_call items.remove-hyperlink on the canary is refused (readOnly)",
        "error: {error}, text {}",
        cut(&text)
    );
    Ok(())
}
