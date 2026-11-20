//! The `polarion` binary end to end against a mock SOAP server.

use std::io::Write as _;
use std::path::PathBuf;
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
    "POLARION_WRITE_TARGETS",
    "POLARION_MCP_CONFIG",
    "POLARION_MCP_TOOLS",
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
    server_with_logouts(1).await
}

/// Mock server that accepts token logins, answers transactions and expects `logouts` logouts
/// (one more for each write of several requests, which runs in a session of its own).
async fn server_with_logouts(logouts: u64) -> MockServer {
    let server = MockServer::start().await;
    for (op, response) in [
        ("beginTransaction>", "<beginTransactionResponse/>"),
        ("endTransaction>", "<endTransactionResponse/>"),
    ] {
        Mock::given(path(SESSION))
            .and(body_string_contains(op))
            .respond_with(xml(response))
            .mount(&server)
            .await;
    }
    Mock::given(path(SESSION))
        .and(body_string_contains("logInWithToken>"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            envelope(
                r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">-1</ns1:sessionID></soapenv:Header>"#,
                "<logInWithTokenResponse/>",
            ),
            "text/xml",
        ))
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

async fn mock(server: &MockServer, needles: &[&str], body: &str) {
    let mut m = Mock::given(path(TRACKER));
    for n in needles {
        m = m.and(body_string_contains(*n));
    }
    m.respond_with(xml(body)).expect(1).mount(server).await;
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }
}

/// Run the binary with `POLARION_URL` pointing at `server` (if any), a token, and extra env vars.
async fn polarion(server: Option<&MockServer>, env: &[(&str, &str)], args: &[&str]) -> Run {
    polarion_with_input(server, env, args, None).await
}

/// [`polarion`], with `input` on standard input (none: standard input is closed).
async fn polarion_with_input(
    server: Option<&MockServer>,
    env: &[(&str, &str)],
    args: &[&str],
    input: Option<&str>,
) -> Run {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    for var in VARS {
        cmd.env_remove(var);
    }
    if let Some(server) = server {
        cmd.env("POLARION_URL", format!("{}/polarion", server.uri()));
        cmd.env("POLARION_TOKEN", "t");
    }
    cmd.envs(env.iter().copied()).args(args);
    let input = input.map(str::to_owned);
    let out = tokio::task::spawn_blocking(move || match input {
        None => cmd.output(),
        Some(input) => {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            // Dropped at the end of the statement: the binary sees the end of its input.
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        }
    })
    .await
    .unwrap()
    .expect("binary runs");
    Run {
        code: out.status.code().expect("exit code"),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn item_show_prints_the_json_form() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>", "projectId>P<", ">P-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1">
<customFields><Custom><key>asil</key><value xsi:type="ns2:EnumOptionId" xmlns:ns2="http://ws.polarion.com/TrackerWebService-types"><id>D</id></value></Custom></customFields>
<id>P-1</id>
<linkedWorkItems><LinkedWorkItem><role><id>parent</id></role><workItemURI>{URI}P${{WorkItem}}P-0</workItemURI></LinkedWorkItem></linkedWorkItems>
<status><id>open</id></status>
<title>Brakes</title>
<type><id>requirement</id></type>
</getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;

    let run = polarion(Some(&server), &[], &["items", "get", "P/P-1", "--json"]).await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.json(),
        json!({
            "id": "P-1",
            "project": "P",
            "type": "requirement",
            "title": "Brakes",
            "status": "open",
            "links": [{"role": "parent", "item": {"project": "P", "id": "P-0"}}],
            "customFields": {"asil": "D"},
            "uri": format!("{URI}P${{WorkItem}}P-1"),
        })
    );
    // Not a terminal: one line.
    assert_eq!(run.stdout.trim_end().lines().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_items_exit_with_3() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemById>"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-9" unresolvable="true"/></getWorkItemByIdResponse>"#
        ),
    )
    .await;

    let run = polarion(
        Some(&server),
        &[("POLARION_PROJECT", "P")],
        &["--json", "items", "get", "P-9"],
    )
    .await;
    assert_eq!(run.code, 3);
    assert!(run.stdout.is_empty());
    let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "notFound");
    assert_eq!(error["error"]["message"], "work item `P/P-9` not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn item_query_prints_a_table_and_a_limit_note() {
    let server = server().await;
    mock(
        &server,
        &[
            "queryWorkItemsLimited>",
            ">project.id:P AND (type:requirement)<",
            "resultsLimit>2<",
            "fields>customFields.asil<",
        ],
        r#"<queryWorkItemsLimitedResponse>
<queryWorkItemsLimitedReturn><id>P-1</id><status><id>open</id></status><title>Brakes</title><type><id>requirement</id></type></queryWorkItemsLimitedReturn>
<queryWorkItemsLimitedReturn><id>P-10</id><title>Steering</title><type><id>requirement</id></type></queryWorkItemsLimitedReturn>
</queryWorkItemsLimitedResponse>"#,
    )
    .await;

    let run = polarion(
        Some(&server),
        &[],
        &[
            "-p",
            "P",
            "items",
            "query",
            "type:requirement",
            "--limit",
            "2",
            "--custom",
            "asil",
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "ID    TYPE         STATUS  TITLE\n\
         P-1   requirement  open    Brakes\n\
         P-10  requirement          Steering\n"
    );
    assert!(
        run.stderr.contains("showing the first 2 results"),
        "{}",
        run.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn doc_show_skips_the_body_unless_asked() {
    let server = server().await;
    mock(
        &server,
        &["getModuleByLocationWithFields>", "projectId>P<", "location>Specs/SRS<"],
        &format!(
            r#"<getModuleByLocationWithFieldsResponse><getModuleByLocationWithFieldsReturn uri="{URI}P${{Module}}{{moduleFolder}}Specs#SRS"><moduleFolder>Specs</moduleFolder><moduleName>SRS</moduleName><title>System requirements</title></getModuleByLocationWithFieldsReturn></getModuleByLocationWithFieldsResponse>"#
        ),
    )
    .await;

    let run = polarion(
        Some(&server),
        &[("POLARION_PROJECT", "P")],
        &["documents", "get", "Specs/SRS"],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "project: P\nspace: Specs\nname: SRS\ntitle: System requirements\nuri: {URI}P${{Module}}{{moduleFolder}}Specs#SRS\n"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_errors_exit_with_1_before_any_request() {
    let run = polarion(None, &[], &["--json", "items", "get", "P-1"]).await;
    assert_eq!(run.code, 1);
    let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "config");
    assert!(run.stderr.contains("POLARION_URL"));

    let run = polarion(
        None,
        &[
            ("POLARION_URL", "http://127.0.0.1:9"),
            ("POLARION_TOKEN", "t"),
        ],
        &["items", "get", "P-1"],
    )
    .await;
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("no project"), "{}", run.stderr);

    let run = polarion(None, &[], &["items", "frobnicate"]).await;
    assert_eq!(run.code, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_are_listed() {
    let run = polarion(None, &[], &["commands", "plans"]).await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.starts_with("COMMAND"), "{}", run.stdout);
    assert!(run.stdout.contains("plans statistics"));
    assert!(
        run.stdout
            .lines()
            .any(|l| l.starts_with("plans add-items") && l.contains(" write ")),
        "{}",
        run.stdout
    );

    let run = polarion(None, &[], &["--json", "commands", "builds"]).await;
    let list: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(list[0]["name"], "builds.all");
    assert_eq!(list[2]["input"]["required"], json!(["query"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_are_refused_without_write_projects() {
    let server = MockServer::start().await;
    let run = polarion(Some(&server), &[], &["--json", "items", "delete", "P/P-1"]).await;
    assert_eq!(run.code, 1);
    let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "readOnly");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_written_to_a_file() {
    let server = server().await;
    mock(
        &server,
        &["getAttachment>", "attachmentId>a-1<"],
        "<getAttachmentResponse><getAttachmentReturn>aGVsbG8=</getAttachmentReturn></getAttachmentResponse>",
    )
    .await;
    let dir = std::env::temp_dir().join(format!("polarion-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.bin");

    let run = polarion(
        Some(&server),
        &[],
        &[
            "items",
            "attachment",
            "P/P-1",
            "--attachment",
            "a-1",
            "--out",
            file.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(std::fs::read(&file).unwrap(), b"hello");
    assert!(run.stderr.contains("wrote 5 bytes"));
    std::fs::remove_dir_all(dir).unwrap();
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
    let run = polarion(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &["items", "add-assignee", "Sandbox/SB-1", "--user", "jdoe"],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "changed\n");
}

// ---- write targets

const ADDED: &str =
    "<addAssigneeResponse><addAssigneeReturn>true</addAssigneeReturn></addAssigneeResponse>";

#[tokio::test(flavor = "multi_thread")]
async fn writes_go_only_to_write_targets() {
    // The target, resolved in POLARION_PROJECT; write projects have no effect.
    let server = server().await;
    mock(
        &server,
        &[
            "addAssignee>",
            &format!("{URI}Sandbox${{WorkItem}}SB-1<"),
            "assigneeId>jdoe<",
        ],
        ADDED,
    )
    .await;
    let run = polarion(
        Some(&server),
        &[
            ("POLARION_PROJECT", "Sandbox"),
            ("POLARION_WRITE_TARGETS", "SB-1, doc:Specs/SRS"),
            ("POLARION_WRITE_PROJECTS", "Other"),
        ],
        &["items", "add-assignee", "SB-1", "--user", "jdoe"],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "changed\n");

    // --write-targets is resolved in the project of -p.
    let other = self::server().await;
    mock(
        &other,
        &[
            "addAssignee>",
            &format!("{URI}Other${{WorkItem}}OT-1<"),
            "assigneeId>jdoe<",
        ],
        ADDED,
    )
    .await;
    let run = polarion(
        Some(&other),
        &[("POLARION_PROJECT", "Sandbox")],
        &[
            "-p",
            "Other",
            "--write-targets",
            "OT-1",
            "items",
            "add-assignee",
            "OT-1",
            "--user",
            "jdoe",
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "changed\n");

    // Everything else is refused before anything is sent, not even a login.
    let untouched = MockServer::start().await;
    let assign = |item: &'static str| ["items", "add-assignee", item, "--user", "jdoe"];
    for (env, args) in [
        // Another item of the same project, one the write projects would allow.
        (
            &[
                ("POLARION_WRITE_TARGETS", "Sandbox/SB-1"),
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ][..],
            assign("Sandbox/SB-2").to_vec(),
        ),
        // The target, but a write with no bounds, and a create.
        (
            &[("POLARION_WRITE_TARGETS", "Sandbox/SB-1")][..],
            vec!["items", "delete", "Sandbox/SB-1"],
        ),
        (
            &[("POLARION_WRITE_TARGETS", "Sandbox/SB-1")][..],
            vec![
                "items",
                "create",
                "--project",
                "Sandbox",
                "--type",
                "task",
                "--title",
                "New",
            ],
        ),
        // Set but empty: no writes at all, whatever else allows them.
        (
            &[
                ("POLARION_WRITE_TARGETS", ""),
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
                ("POLARION_ALLOW_GLOBAL_WRITES", "1"),
            ][..],
            assign("Sandbox/SB-1").to_vec(),
        ),
        // The option wins over the environment, also when it is empty.
        (
            &[("POLARION_WRITE_TARGETS", "Sandbox/SB-1")][..],
            [
                &["--write-targets", "Sandbox/SB-2"][..],
                &assign("Sandbox/SB-1")[..],
            ]
            .concat(),
        ),
        (
            &[("POLARION_WRITE_PROJECTS", "Sandbox")][..],
            [&["--write-targets="][..], &assign("Sandbox/SB-1")[..]].concat(),
        ),
        // Only commas and blanks: no targets, no writes.
        (
            &[
                ("POLARION_WRITE_TARGETS", "  ,  , "),
                ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ][..],
            assign("Sandbox/SB-1").to_vec(),
        ),
        (
            &[("POLARION_WRITE_PROJECTS", "Sandbox")][..],
            [&["--write-targets", " , "][..], &assign("Sandbox/SB-1")[..]].concat(),
        ),
        // -p does not move the environment's targets: SB-1 there is Sandbox/SB-1, so the
        // command's SB-1 (Other/SB-1) is no target.
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "SB-1"),
            ][..],
            [&["-p", "Other"][..], &assign("SB-1")[..]].concat(),
        ),
        // A comma inside a document name, escaped: the document is the only target.
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", r"doc:Specs/Brakes\, SB-1"),
            ][..],
            assign("SB-1").to_vec(),
        ),
    ] {
        let run = polarion(
            Some(&untouched),
            env,
            &[&["--json"][..], &args[..]].concat(),
        )
        .await;
        assert_eq!(run.code, 1, "{args:?}: {}", run.stderr);
        let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
        assert_eq!(error["error"]["code"], "readOnly", "{args:?}: {error}");
    }
    assert!(
        untouched.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_targets_that_do_not_resolve_are_configuration_errors() {
    let untouched = MockServer::start().await;
    for (env, args, message) in [
        (
            &[("POLARION_WRITE_TARGETS", "Sandbox/SB-1, Specs/SRS/x")][..],
            &[][..],
            "invalid write target `Specs/SRS/x`",
        ),
        // No default project to resolve the id in: none, an empty one, or only -p (which
        // applies to --write-targets, not to the environment's).
        (
            &[("POLARION_WRITE_TARGETS", "SB-1")][..],
            &[][..],
            "has no project",
        ),
        (
            &[("POLARION_WRITE_TARGETS", "SB-1"), ("POLARION_PROJECT", "")][..],
            &[][..],
            "has no project",
        ),
        (
            &[
                ("POLARION_WRITE_TARGETS", "SB-1"),
                ("POLARION_PROJECT", " "),
            ][..],
            &["-p", ""][..],
            "has no project",
        ),
        (
            &[("POLARION_WRITE_TARGETS", "SB-1")][..],
            &["-p", "Sandbox"][..],
            "has no project",
        ),
        (
            &[("POLARION_PROJECT", "")][..],
            &["--write-targets", "SB-1"][..],
            "has no project",
        ),
        // The rest of a document name cut at an unescaped comma.
        (
            &[
                ("POLARION_PROJECT", "Sandbox"),
                ("POLARION_WRITE_TARGETS", "doc:Specs/Brakes, v2"),
            ][..],
            &[][..],
            "invalid write target `v2`",
        ),
    ] {
        // Reads included: the client is never built.
        let run = polarion(
            Some(&untouched),
            env,
            &[&["--json"][..], args, &["items", "get", "Sandbox/SB-1"][..]].concat(),
        )
        .await;
        assert_eq!(run.code, 1, "{}", run.stderr);
        let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
        assert_eq!(error["error"]["code"], "config", "{error}");
        let text = error["error"]["message"].as_str().unwrap();
        assert!(text.contains(message), "{text}");
    }
    assert!(untouched.received_requests().await.unwrap().is_empty());
}

/// `SB-` followed by text that is not Unicode.
fn not_unicode() -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        std::ffi::OsString::from_vec(b"SB-\xff1".to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt as _;
        std::ffi::OsString::from_wide(&[0x53, 0x42, 0x2d, 0xd800, 0x31])
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn write_targets_that_are_not_unicode_are_configuration_errors() {
    // As in the MCP server: a `config` error, not a usage error, and nothing is sent.
    let untouched = MockServer::start().await;
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    for var in VARS {
        cmd.env_remove(var);
    }
    cmd.env("POLARION_URL", format!("{}/polarion", untouched.uri()))
        .env("POLARION_TOKEN", "t")
        .env("POLARION_PROJECT", "Sandbox")
        .env("POLARION_WRITE_TARGETS", not_unicode())
        .args(["--json", "items", "add-assignee", "SB-1", "--user", "jdoe"]);
    let out = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .unwrap()
        .expect("binary runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    let error: Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "config", "{error}");
    assert!(untouched.received_requests().await.unwrap().is_empty());
}

// ---- descriptions as Markdown

/// Markdown and the description HTML it converts to (and back from).
const MARKDOWN: &str = "### Brakes\n\nMust stop the car.\n";
const HTML: &str = r#"<span style="font-weight: bold; font-size: 14pt; line-height: 1.2;">Brakes</span><br/><br/>Must stop the car."#;

/// The file name of the diagram `graph TD\n    A-->B` (pinned by the conversion's tests).
const DIAGRAM: &str = "mermaid-744d43fa10d8c687.svg";

/// A fresh directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("polarion-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `Sandbox/SB-1` as `getWorkItemByIdsWithFields` returns it, with a description (`(type,
/// content)`) and attachments (`(id, file name)`).
fn work_item(description: Option<(&str, &str)>, attachments: &[(&str, &str)]) -> String {
    let mut fields = "<id>SB-1</id>".to_owned();
    if let Some((kind, content)) = description {
        fields += &format!(
            "<description><type>{kind}</type><content>{}</content></description>",
            escape(content)
        );
    }
    if !attachments.is_empty() {
        fields += "<attachments>";
        for (id, file_name) in attachments {
            fields +=
                &format!("<Attachment><fileName>{file_name}</fileName><id>{id}</id></Attachment>");
        }
        fields += "</attachments>";
    }
    format!(
        r#"<getWorkItemByIdsWithFieldsResponse><getWorkItemByIdsWithFieldsReturn uri="{URI}Sandbox${{WorkItem}}SB-1">{fields}</getWorkItemByIdsWithFieldsReturn></getWorkItemByIdsWithFieldsResponse>"#
    )
}

/// Answer the `n`-th read of the work item (counting from 1) with `body`.
async fn read(server: &MockServer, n: u8, body: &str) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByIdsWithFields>"))
        .respond_with(xml(body))
        .up_to_n_times(1)
        .with_priority(n)
        .expect(1)
        .mount(server)
        .await;
}

/// The bodies of the `operation` requests the server received, XML entities decoded.
async fn bodies(server: &MockServer, operation: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|body| body.contains(&format!("{operation}>")))
        .map(|body| unescape(&body))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_description_is_printed_as_markdown() {
    for json in [false, true] {
        let server = server().await;
        mock(
            &server,
            &["getWorkItemByIdsWithFields>", "workitemId>SB-1<"],
            &work_item(Some(("text/html", HTML)), &[]),
        )
        .await;
        let mut args = vec!["items", "description-markdown", "Sandbox/SB-1"];
        if json {
            args.push("--json");
        }

        let run = polarion(Some(&server), &[], &args).await;
        assert_eq!(run.code, 0, "{}", run.stderr);
        if json {
            // A JSON string.
            let markdown = run.json();
            assert_eq!(
                markdown.as_str().map(|m| m.trim_end_matches('\n')),
                Some(MARKDOWN.trim_end_matches('\n')),
                "{markdown}"
            );
        } else {
            // As it is, line breaks included, with one at the end.
            assert_eq!(run.stdout, MARKDOWN);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_from_a_file_is_written_only_with_write_projects() {
    let dir = TempDir::new("markdown");
    let file = dir.0.join("desc.md");
    std::fs::write(
        &file,
        "# Brakes\n\nStops the car, see SB-2 (not UTF-8).\n\n```mermaid\ngraph TD\n    A-->B\n```\n",
    )
    .unwrap();
    let from_file = format!("@{}", file.display());
    let args = [
        "items",
        "set-description-markdown",
        "Sandbox/SB-1",
        "--markdown",
        &from_file,
    ];

    // Read-only: refused before anything is sent.
    let untouched = MockServer::start().await;
    let run = polarion(
        Some(&untouched),
        &[],
        &[&["--json"][..], &args[..]].concat(),
    )
    .await;
    assert_eq!(run.code, 1, "{}", run.stderr);
    let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "readOnly");
    assert!(
        untouched.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );

    // The diagram is rendered, uploaded and shown; reading the attachments again gives its id.
    // One upload, then one update: single writes, on the client's session.
    let server = server().await;
    let attachment = format!("1-{DIAGRAM}");
    read(&server, 1, &work_item(None, &[])).await;
    read(&server, 2, &work_item(None, &[(&attachment, DIAGRAM)])).await;
    mock(
        &server,
        &["createAttachment>", &format!("fileName>{DIAGRAM}<")],
        "<createAttachmentResponse/>",
    )
    .await;
    mock(
        &server,
        &["updateWorkItem>", &format!("workitemimg:{attachment}")],
        "<updateWorkItemResponse/>",
    )
    .await;
    let run = polarion(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &args,
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "updated: true\ndiagrams:\n  -\n    fileName: {DIAGRAM}\n    attachment: {attachment}\n    uploaded: true\n"
        )
    );
    // Ids with the work item's own prefix are links, others are text.
    let update = &bodies(&server, "updateWorkItem").await[0];
    assert!(
        update.contains(r#"data-item-id="SB-2""#) && !update.contains(r#"data-item-id="UTF-8""#),
        "{update}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_is_appended_from_standard_input() {
    let server = server().await;
    mock(
        &server,
        &["getWorkItemByIdsWithFields>"],
        &work_item(Some(("text/html", "<b>old</b>")), &[]),
    )
    .await;
    mock(&server, &["updateWorkItem>"], "<updateWorkItemResponse/>").await;

    let run = polarion_with_input(
        Some(&server),
        &[("POLARION_WRITE_PROJECTS", "Sandbox")],
        &[
            "items",
            "append-description-markdown",
            "Sandbox/SB-1",
            "--markdown",
            "@-",
        ],
        Some("**new**\n"),
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "updated: true\n");
    let update = &bodies(&server, "updateWorkItem").await[0];
    assert!(
        update.contains(r#"<b>old</b><br/><br/><span style="font-weight: bold;">new</span>"#),
        "{update}"
    );
}

/// Markdown often starts with `-`: a list, a rule.
#[tokio::test(flavor = "multi_thread")]
async fn markdown_that_starts_with_a_hyphen_is_the_value() {
    for markdown in ["- first item\n- second item", "---", "-1 degree", "-- x"] {
        let server = server().await;
        mock(
            &server,
            &["getWorkItemByIdsWithFields>"],
            &work_item(Some(("text/html", "<b>old</b>")), &[]),
        )
        .await;
        mock(&server, &["updateWorkItem>"], "<updateWorkItemResponse/>").await;

        let run = polarion(
            Some(&server),
            &[("POLARION_WRITE_PROJECTS", "Sandbox")],
            &[
                "items",
                "append-description-markdown",
                "Sandbox/SB-1",
                "--markdown",
                markdown,
            ],
        )
        .await;
        assert_eq!(run.code, 0, "{markdown:?}: {}", run.stderr);
        let updates = bodies(&server, "updateWorkItem").await;
        assert_eq!(updates.len(), 1, "{markdown:?}");
        if markdown.starts_with("- ") {
            assert!(
                updates[0].contains("<li") && updates[0].contains("second item"),
                "{}",
                updates[0]
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_is_saved_with_out_and_an_empty_description_is_no_error() {
    let dir = TempDir::new("markdown-out");
    let file = dir.0.join("desc.md");
    let described = server().await;
    mock(
        &described,
        &["getWorkItemByIdsWithFields>"],
        &work_item(Some(("text/html", HTML)), &[]),
    )
    .await;
    let out = file.to_str().unwrap();
    let run = polarion(
        Some(&described),
        &[],
        &[
            "items",
            "description-markdown",
            "Sandbox/SB-1",
            "--out",
            out,
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("wrote "), "{}", run.stderr);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), MARKDOWN);

    // No description: empty Markdown, not "no results".
    let empty = server().await;
    mock(
        &empty,
        &["getWorkItemByIdsWithFields>"],
        &work_item(None, &[]),
    )
    .await;
    let run = polarion(
        Some(&empty),
        &[],
        &["items", "description-markdown", "Sandbox/SB-1"],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert!(!run.stderr.contains("no results"), "{}", run.stderr);
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_a_workflow_requires_are_written_with_the_create() {
    // The create, its category and its link: one transaction, in a session of its own.
    let server = server_with_logouts(2).await;
    Mock::given(path("/polarion/ws/services/ProjectWebService"))
        .and(body_string_contains("getProject>"))
        .respond_with(xml(
            r#"<getProjectResponse><getProjectReturn uri="subterra:data-service:objects:/default/Sandbox${Project}Sandbox"><id>Sandbox</id></getProjectReturn></getProjectResponse>"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let new = "subterra:data-service:objects:/default/Sandbox${WorkItem}SB-9";
    mock(
        &server,
        &["createWorkItem>", "type><trackert:id>task<"],
        &format!("<createWorkItemResponse><createWorkItemReturn>{new}</createWorkItemReturn></createWorkItemResponse>"),
    )
    .await;
    mock(
        &server,
        &["addCategory>", new, "categoryId>software<"],
        "<addCategoryResponse><addCategoryReturn>true</addCategoryReturn></addCategoryResponse>",
    )
    .await;
    mock(
        &server,
        &["addLinkedItem>", new, "role><trackert:id>parent<"],
        "<addLinkedItemResponse><addLinkedItemReturn>true</addLinkedItemReturn></addLinkedItemResponse>",
    )
    .await;
    let run = polarion(
        Some(&server),
        &[
            ("POLARION_WRITE_PROJECTS", "Sandbox"),
            ("POLARION_PROJECT", "Sandbox"),
        ],
        &[
            "--json",
            "items",
            "create",
            "--type",
            "task",
            "--title",
            "Brakes",
            "--categories",
            "software",
            "--links",
            "parent=Sandbox/SB-1",
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    let created: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(created["uri"], new);
    let operations: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| {
            let body = String::from_utf8_lossy(&r.body);
            let rest = body.split_once("<soapenv:Body><")?.1.to_owned();
            let element = rest.split([' ', '>', '/']).next()?.to_owned();
            Some(element.rsplit(':').next()?.to_owned())
        })
        .collect();
    assert_eq!(
        operations,
        [
            "logInWithToken",
            "getProject",
            "logInWithToken",
            "beginTransaction",
            "createWorkItem",
            "addCategory",
            "addLinkedItem",
            "endTransaction",
            "endSession",
            "endSession",
        ]
    );
}

// ---- Markdown files with images

/// The first bytes of a PNG file, and some more.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimage";

#[tokio::test(flavor = "multi_thread")]
async fn a_markdown_file_is_written_with_its_images() {
    let dir = TempDir::new("markdown-file");
    std::fs::create_dir_all(dir.0.join("img")).unwrap();
    std::fs::write(dir.0.join("img/a.png"), PNG).unwrap();
    let file = dir.0.join("desc.md");
    std::fs::write(&file, "Shot: ![s](img/a.png)\n").unwrap();
    let file = file.to_str().unwrap();
    let write = [("POLARION_WRITE_PROJECTS", "Sandbox")];

    // The image is attached, and the description shows the attachment.
    let server = server().await;
    read(&server, 1, &work_item(None, &[])).await;
    read(&server, 2, &work_item(None, &[("1-a.png", "a.png")])).await;
    mock(
        &server,
        &["createAttachment>", "fileName>a.png<"],
        "<createAttachmentResponse/>",
    )
    .await;
    mock(
        &server,
        &["updateWorkItem>", "workitemimg:1-a.png"],
        "<updateWorkItemResponse/>",
    )
    .await;
    let run = polarion(
        Some(&server),
        &write,
        &[
            "items",
            "set-description-markdown",
            "Sandbox/SB-1",
            "--file",
            file,
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "updated: true\nimages:\n  -\n    url: img/a.png\n    attachment: 1-a.png\n    uploaded: true\n"
    );
    let created = &bodies(&server, "createAttachment").await[0];
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(PNG);
    assert!(created.contains(&data), "{created}");

    // Refused before anything is sent: both sources of the Markdown, none, and an image that
    // is missing or not in the folder of the file.
    std::fs::write(dir.0.join("missing.md"), "![](img/none.png)").unwrap();
    std::fs::write(dir.0.join("img/up.md"), "![](../img/a.png)").unwrap();
    let missing = dir.0.join("missing.md");
    let up = dir.0.join("img/up.md");
    let untouched = MockServer::start().await;
    for (args, why) in [
        (vec!["--file", file, "--markdown", "text"], "not both"),
        (vec![], "give the Markdown"),
        (vec!["--file", missing.to_str().unwrap()], "none.png"),
        (
            vec!["--file", up.to_str().unwrap()],
            "folder of the Markdown file",
        ),
    ] {
        for command in ["set-description-markdown", "append-description-markdown"] {
            let args = [&["--json", "items", command, "Sandbox/SB-1"][..], &args[..]].concat();
            let run = polarion(Some(&untouched), &write, &args).await;
            assert_eq!(run.code, 1, "{args:?}: {}", run.stderr);
            let error: Value = serde_json::from_str(run.stderr.trim()).unwrap();
            assert_eq!(error["error"]["code"], "invalid", "{error}");
            let message = error["error"]["message"].as_str().unwrap();
            assert!(message.contains(why), "{message}");
        }
    }
    assert!(
        untouched.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_description_is_exported_to_a_folder() {
    use base64::Engine as _;
    let dir = TempDir::new("export");
    let out = dir.0.join("out");
    let server = server().await;
    let html = r#"Text <img src="workitemimg:1-a.png" title="a.png" style="max-width: 650px;"/>"#;
    mock(
        &server,
        &["getWorkItemByIdsWithFields>", "workitemId>SB-1<"],
        &work_item(Some(("text/html", html)), &[("1-a.png", "a.png")]),
    )
    .await;
    mock(
        &server,
        &["getAttachment>", "attachmentId>1-a.png<"],
        &format!(
            "<getAttachmentResponse><getAttachmentReturn>{}</getAttachmentReturn></getAttachmentResponse>",
            base64::engine::general_purpose::STANDARD.encode(PNG)
        ),
    )
    .await;

    // Reading needs no write scope.
    let run = polarion(
        Some(&server),
        &[],
        &[
            "--json",
            "items",
            "export-description",
            "Sandbox/SB-1",
            "--dir",
            out.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    let markdown = out.join("SB-1.md");
    let image = out.join("SB-1").join("1-a.png");
    assert_eq!(
        run.json(),
        json!({
            "markdown": markdown.display().to_string(),
            "images": [image.display().to_string()],
        })
    );
    assert_eq!(
        std::fs::read_to_string(&markdown).unwrap(),
        "Text ![](SB-1/1-a.png \"a.png\")"
    );
    assert_eq!(std::fs::read(&image).unwrap(), PNG);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_is_a_command_of_the_binary() {
    // Its help comes from the server's crate; its version is the binary's.
    let run = polarion(None, &[], &["mcp", "--help"]).await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    for expected in [
        "--http <ADDR>",
        "--config <FILE>",
        "POLARION_MCP_AUTH_TOKEN",
    ] {
        assert!(run.stdout.contains(expected), "{expected}: {}", run.stdout);
    }
    let run = polarion(None, &[], &["mcp", "--version"]).await;
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.trim().ends_with(env!("CARGO_PKG_VERSION")),
        "{}",
        run.stdout
    );
    // What only applies to a command's result is refused, not ignored.
    for args in [&["--json", "mcp"][..], &["mcp", "--out", "x"]] {
        let run = polarion(None, &[], args).await;
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stderr.contains("do not apply"), "{}", run.stderr);
        assert_eq!(run.stdout, "", "{args:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn boolean_settings_given_empty_are_off_and_unclear_ones_are_refused() {
    // An empty value is what `-e VAR=` or a profile's `${VAR-}` passes on.
    for var in ["POLARION_INSECURE", "POLARION_ALLOW_GLOBAL_WRITES"] {
        let run = polarion(None, &[(var, "")], &["commands", "session"]).await;
        assert_eq!(run.code, 0, "{var}: {}", run.stderr);
        // The server: the command line is accepted (what stops it is `--json`, checked after).
        let run = polarion(None, &[(var, "")], &["mcp", "--json"]).await;
        assert!(run.stderr.contains("do not apply"), "{var}: {}", run.stderr);
        for args in [&["commands", "session"][..], &["mcp", "--json"]] {
            let run = polarion(None, &[(var, "maybe")], args).await;
            assert_eq!(run.code, 2, "{var} {args:?}");
            assert!(
                run.stderr.contains("invalid value 'maybe'"),
                "{}",
                run.stderr
            );
        }
    }
}
