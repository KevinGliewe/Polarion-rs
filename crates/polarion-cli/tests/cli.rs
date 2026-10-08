//! The `polarion` binary end to end against a mock SOAP server.

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
        ))
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
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_polarion"));
    for var in VARS {
        cmd.env_remove(var);
    }
    if let Some(server) = server {
        cmd.env("POLARION_URL", format!("{}/polarion", server.uri()));
        cmd.env("POLARION_TOKEN", "t");
    }
    cmd.envs(env.iter().copied()).args(args);
    let out = tokio::task::spawn_blocking(move || cmd.output())
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
            r#"<getModuleByLocationWithFieldsResponse><getModuleByLocationWithFieldsReturn uri="{URI}P${{Module}}{{moduleFolder}}Specs/SRS"><moduleFolder>Specs</moduleFolder><moduleName>SRS</moduleName><title>System requirements</title></getModuleByLocationWithFieldsReturn></getModuleByLocationWithFieldsResponse>"#
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
            "project: P\nspace: Specs\nname: SRS\ntitle: System requirements\nuri: {URI}P${{Module}}{{moduleFolder}}Specs/SRS\n"
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
