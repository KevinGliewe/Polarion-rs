//! Writes of several requests: one transaction in a Polarion session of its own (`atomic.rs`),
//! seen from the requests each session sends. The mock gives every login a session of its own
//! (`-1` for the client's, `-2`, ... for the sessions of their own).

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::*;
use polarion::soap::Error as SoapError;
use polarion::{
    DocumentRef, Error, FieldValue, ItemRef, NewDocument, NewExternalLink, NewHyperlink, NewLink,
    NewLinkedRevision, NewOslcLink, NewPlan, NewPlanningConstraint, NewTestRun, NewWorkItem,
    PlanUpdate, Polarion, TestRunUpdate, Text, WorkItemUpdate,
};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sandbox(id: &str) -> ItemRef {
    ItemRef::new("Sandbox", id)
}

fn owned(log: &[(&str, &str)]) -> Vec<(String, String)> {
    log.iter()
        .map(|(s, o)| ((*s).to_owned(), (*o).to_owned()))
        .collect()
}

/// The operations of a log.
fn operations(log: &[(String, String)]) -> Vec<&str> {
    log.iter().map(|(_, o)| o.as_str()).collect()
}

/// An update of three writes: `updateWorkItem`, `setCustomField` and `setFieldsNull`.
fn three_writes() -> WorkItemUpdate {
    WorkItemUpdate {
        title: Some("New title".into()),
        custom_fields: BTreeMap::from([("risk".to_owned(), Some(FieldValue::Int(3)))]),
        clear: vec!["dueDate".into()],
        ..WorkItemUpdate::default()
    }
}

/// Answers the three writes of [`three_writes`]; `custom` answers `setCustomField`.
async fn answer_update(server: &MockServer, custom: ResponseTemplate) {
    for (op, response) in [
        ("updateWorkItem>", xml("<updateWorkItemResponse/>")),
        ("setCustomField>", custom),
        ("setFieldsNull>", xml("<setFieldsNullResponse/>")),
    ] {
        Mock::given(path(TRACKER))
            .and(body_string_contains(op))
            .respond_with(response)
            .mount(server)
            .await;
    }
}

fn custom_set() -> ResponseTemplate {
    xml("<setCustomFieldResponse/>")
}

/// Answers a session operation (`endTransaction`, ...) before the defaults of `session_server()`.
async fn answer_session(server: &MockServer, operation: &str, response: ResponseTemplate) {
    Mock::given(path(SESSION))
        .and(body_string_contains(format!("{operation}>")))
        .respond_with(response)
        .with_priority(1)
        .mount(server)
        .await;
}

async fn update(polarion: &Polarion) -> polarion::Result<()> {
    polarion
        .items()
        .update(&sandbox("SB-1"), &three_writes())
        .await
}

/// The log once its last request is `endSession` (the transaction of a dropped caller goes on
/// in its task); fails after about two seconds.
async fn log_until_ended(server: &MockServer) -> Vec<(String, String)> {
    for _ in 0..100 {
        let log = log(server).await;
        if log.last().is_some_and(|(_, o)| o == "endSession") {
            return log;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the session was not ended: {:?}", log(server).await);
}

#[tokio::test]
async fn updates_of_several_writes_run_in_one_transaction_of_their_own() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    let polarion = writer(&server);
    polarion.login().await.unwrap();

    update(&polarion).await.unwrap();
    assert_eq!(
        log(&server).await,
        owned(&[
            ("", "logInWithToken"),
            ("", "logInWithToken"),
            ("-2", "beginTransaction"),
            ("-2", "updateWorkItem"),
            ("-2", "setCustomField"),
            ("-2", "setFieldsNull"),
            ("-2", "endTransaction"),
            ("-2", "endSession"),
        ])
    );
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>false<"), "{}", ends[0]);
    // The client's session is untouched.
    assert_eq!(polarion.soap().session_id().as_deref(), Some("-1"));
}

#[tokio::test]
async fn single_writes_never_open_a_session_of_their_own() {
    let server = session_server().await;
    for (op, response) in [
        ("updateWorkItem>", "<updateWorkItemResponse/>"),
        (
            "addComment>",
            "<addCommentResponse><addCommentReturn>c1</addCommentReturn></addCommentResponse>",
        ),
    ] {
        Mock::given(path(TRACKER))
            .and(body_string_contains(op))
            .respond_with(xml(response))
            .mount(&server)
            .await;
    }
    let polarion = writer(&server);
    let items = polarion.items();

    let update = WorkItemUpdate {
        title: Some("New title".into()),
        ..WorkItemUpdate::default()
    };
    items.update(&sandbox("SB-1"), &update).await.unwrap();
    let text = Text {
        content_type: "text/plain".into(),
        content: "Note".into(),
    };
    items
        .add_comment(&sandbox("SB-1"), None, &text)
        .await
        .unwrap();

    assert_eq!(
        log(&server).await,
        owned(&[
            ("", "logInWithToken"),
            ("-1", "updateWorkItem"),
            ("-1", "addComment"),
        ])
    );
}

#[tokio::test]
async fn failed_writes_roll_back_and_end_the_session() {
    let server = session_server().await;
    answer_update(&server, fault("no such field")).await;

    let result = update(&writer(&server)).await;
    assert!(
        matches!(result, Err(Error::Soap(SoapError::Fault { .. }))),
        "{result:?}"
    );
    let log = log(&server).await;
    assert_eq!(
        operations(&log),
        [
            "logInWithToken",
            "beginTransaction",
            "updateWorkItem",
            "setCustomField",
            // A fault is checked against the session, which is valid: no renewal.
            "hasSubject",
            "endTransaction",
            "endSession",
        ]
    );
    // `setFieldsNull` was never sent; the rollback discards `updateWorkItem`.
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>true<"), "{}", ends[0]);
    assert!(log[1..].iter().all(|(s, _)| s == "-1"), "{log:?}");
}

#[tokio::test]
async fn refused_commits_return_the_fault() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    answer_session(&server, "endTransaction", fault("conflict")).await;

    let result = update(&writer(&server)).await;
    assert!(
        matches!(result, Err(Error::Soap(SoapError::Fault { .. }))),
        "{result:?}"
    );
    let log = log(&server).await;
    let ops = operations(&log);
    assert_eq!(ops.iter().filter(|o| **o == "endTransaction").count(), 1);
    assert_eq!(ops.last(), Some(&"endSession"), "{ops:?}");
}

#[tokio::test]
async fn unanswered_commits_are_uncertain() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    answer_session(
        &server,
        "endTransaction",
        ResponseTemplate::new(502).set_body_string("Bad Gateway"),
    )
    .await;

    let result = update(&writer(&server)).await;
    assert!(
        matches!(
            result,
            Err(Error::CommitUncertain(SoapError::Status { .. }))
        ),
        "{result:?}"
    );
    let log = log(&server).await;
    let ops = operations(&log);
    assert_eq!(ops.iter().filter(|o| **o == "endTransaction").count(), 1);
    assert_eq!(ops.last(), Some(&"endSession"), "{ops:?}");
}

#[tokio::test]
async fn failed_begins_send_no_write() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    answer_session(&server, "beginTransaction", fault("no")).await;

    let result = update(&writer(&server)).await;
    assert!(
        matches!(result, Err(Error::Soap(SoapError::Fault { .. }))),
        "{result:?}"
    );
    assert_eq!(
        operations(&log(&server).await),
        [
            "logInWithToken",
            "beginTransaction",
            "hasSubject",
            "endSession"
        ]
    );
}

#[tokio::test]
async fn expiry_inside_the_transaction_commits_nothing() {
    let server = session_server().await;
    answer_update(&server, fault("Not authorized.")).await;
    answer_session(&server, "hasSubject", subject(false)).await;
    let polarion = writer(&server);
    polarion.login().await.unwrap();

    let result = update(&polarion).await;
    assert!(
        matches!(result, Err(Error::Soap(SoapError::SessionExpired))),
        "{result:?}"
    );
    let log = log(&server).await;
    let ops = operations(&log);
    // The client's login and the session's only one: no renewal, no commit, no rollback.
    assert_eq!(ops.iter().filter(|o| **o == "logInWithToken").count(), 2);
    assert!(!ops.contains(&"endTransaction"), "{ops:?}");
    assert!(!ops.contains(&"setFieldsNull"), "{ops:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_callers_still_finish_the_transaction() {
    // Committed: the writes answer after the caller gave up.
    let server = session_server().await;
    answer_update(&server, custom_set().set_delay(Duration::from_millis(300))).await;
    let polarion = writer(&server);
    let call = tokio::time::timeout(Duration::from_millis(50), update(&polarion));
    assert!(call.await.is_err(), "the call was not cut off");
    let log = log_until_ended(&server).await;
    assert_eq!(
        operations(&log),
        [
            "logInWithToken",
            "beginTransaction",
            "updateWorkItem",
            "setCustomField",
            "setFieldsNull",
            "endTransaction",
            "endSession",
        ]
    );
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>false<"), "{}", ends[0]);

    // Rolled back: a write fails after the caller gave up.
    let server = session_server().await;
    answer_update(&server, fault("no").set_delay(Duration::from_millis(300))).await;
    let polarion = writer(&server);
    let call = tokio::time::timeout(Duration::from_millis(50), update(&polarion));
    assert!(call.await.is_err(), "the call was not cut off");
    let log = log_until_ended(&server).await;
    assert_eq!(
        operations(&log),
        [
            "logInWithToken",
            "beginTransaction",
            "updateWorkItem",
            "setCustomField",
            "hasSubject",
            "endTransaction",
            "endSession",
        ]
    );
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>true<"), "{}", ends[0]);
}

#[tokio::test]
async fn explicit_transactions_are_joined() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    let polarion = writer(&server);

    polarion.begin_transaction().await.unwrap();
    update(&polarion).await.unwrap();
    polarion.commit().await.unwrap();
    assert_eq!(
        log(&server).await,
        owned(&[
            ("", "logInWithToken"),
            ("-1", "beginTransaction"),
            ("-1", "updateWorkItem"),
            ("-1", "setCustomField"),
            ("-1", "setFieldsNull"),
            ("-1", "endTransaction"),
        ])
    );
}

#[tokio::test]
async fn concurrent_updates_have_sessions_of_their_own() {
    let server = session_server().await;
    answer_update(&server, custom_set()).await;
    let polarion = writer(&server);
    polarion.login().await.unwrap();

    let (a, b) = tokio::join!(update(&polarion), update(&polarion));
    a.unwrap();
    b.unwrap();
    let log = log(&server).await;
    for session in ["-2", "-3"] {
        let ops: Vec<_> = log
            .iter()
            .filter(|(s, _)| s == session)
            .map(|(_, o)| o.as_str())
            .collect();
        assert_eq!(
            ops,
            [
                "beginTransaction",
                "updateWorkItem",
                "setCustomField",
                "setFieldsNull",
                "endTransaction",
                "endSession"
            ],
            "{log:?}"
        );
    }
    assert!(!log.iter().any(|(s, _)| s == "-1"), "{log:?}");
}

// ---- creates with follow-ups

/// The URI of the work item the server creates.
fn new_item_uri() -> String {
    format!("{URI}Sandbox${{WorkItem}}SB-9")
}

/// The document the server creates.
fn new_document() -> DocumentRef {
    DocumentRef::new("Sandbox", "Specs", "New")
}

/// The URI of the test run the server creates.
fn new_run_uri() -> String {
    format!("{URI}Sandbox${{TestRun}}R9")
}

/// Answers the requests of a create: `getProject` with the project, `createWorkItem` and
/// `createWorkItemInModule` with [`new_item_uri`], every other operation with `true` (which
/// decodes as `true`, a string, or nothing). Below the session mocks of `session_server()`.
struct Creates;

impl Respond for Creates {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        let element = body
            .split_once("<soapenv:Body><")
            .map_or("", |(_, rest)| rest)
            .split([' ', '>', '/'])
            .next()
            .unwrap_or_default();
        let op = element.rsplit(':').next().unwrap_or_default();
        let inner = match op {
            "getProject" => format!(
                r#"<{op}Return uri="{URI}Sandbox${{Project}}Sandbox"><id>Sandbox</id><name>Sandbox</name></{op}Return>"#
            ),
            "createWorkItem" | "createWorkItemInModule" => {
                format!("<{op}Return>{}</{op}Return>", new_item_uri())
            }
            "createDocument" | "createModule" => {
                format!("<{op}Return>{}</{op}Return>", new_document().uri)
            }
            "createTestRun" | "createTestRunWithTitle" => {
                format!("<{op}Return>{}</{op}Return>", new_run_uri())
            }
            "createPlan" => format!("<{op}Return>{URI}Sandbox${{Plan}}rel9</{op}Return>"),
            _ => format!("<{op}Return>true</{op}Return>"),
        };
        xml(&format!("<{op}Response>{inner}</{op}Response>"))
    }
}

async fn answer_creates(server: &MockServer) {
    Mock::given(method("POST"))
        .respond_with(Creates)
        .with_priority(11)
        .mount(server)
        .await;
}

fn cell(s: &str) -> Text {
    Text {
        content_type: "text/plain".into(),
        content: s.into(),
    }
}

/// A new work item with a follow-up of each kind.
fn with_every_follow_up() -> NewWorkItem {
    NewWorkItem {
        categories: vec!["software".into()],
        assignees: vec!["jdoe".into()],
        approvees: vec!["boss".into()],
        hyperlinks: vec![NewHyperlink {
            role: "ref_ext".into(),
            url: "https://example.com".into(),
        }],
        links: vec![
            NewLink::new("parent", sandbox("SB-1")),
            NewLink {
                revision: Some("42".into()),
                suspect: true,
                ..NewLink::new("relates_to", sandbox("SB-2"))
            },
        ],
        planning_constraints: vec![NewPlanningConstraint {
            constraint: "startNoEarlierThan".into(),
            date: "2024-01-31".into(),
        }],
        planned_in: vec![format!("{URI}Sandbox${{Plan}}rel1")],
        test_steps: vec![vec![cell("Do"), cell("Done")]],
        external_links: vec![NewExternalLink {
            role: "relates_to".into(),
            url: "https://other.example.com/item/1".into(),
        }],
        oslc_links: vec![NewOslcLink {
            role: "relates_to".into(),
            url: "https://oslc.example.com/r/1".into(),
            label: "R1".into(),
        }],
        linked_revisions: vec![
            NewLinkedRevision {
                repository: None,
                revision: "1234".into(),
            },
            NewLinkedRevision {
                repository: Some("ext".into()),
                revision: "abc".into(),
            },
        ],
        ..NewWorkItem::new("Sandbox", "task", "Brakes")
    }
}

#[tokio::test]
async fn creates_with_follow_ups_run_in_one_transaction_of_their_own() {
    let server = session_server().await;
    answer_creates(&server).await;
    let polarion = writer(&server);

    let new = with_every_follow_up();
    assert_eq!(new.follow_ups(), 13);
    let created = polarion.items().create(&new).await.unwrap();
    assert_eq!(created, ItemRef::from_uri(new_item_uri()));
    assert_eq!(
        log(&server).await,
        owned(&[
            ("", "logInWithToken"),
            // The client's session only reads the project.
            ("-1", "getProject"),
            ("", "logInWithToken"),
            ("-2", "beginTransaction"),
            ("-2", "createWorkItem"),
            ("-2", "addCategory"),
            ("-2", "addAssignee"),
            ("-2", "addApprovee"),
            ("-2", "addHyperlink"),
            ("-2", "addLinkedItem"),
            ("-2", "addLinkedItemWithRev"),
            ("-2", "addPlaningContraint"),
            ("-2", "addPlanItems"),
            ("-2", "setTestSteps"),
            ("-2", "addExternallyLinkedItem"),
            ("-2", "addLinkedOslcItem"),
            ("-2", "addLinkedRevision"),
            ("-2", "addExternalLinkedRevision"),
            ("-2", "endTransaction"),
            ("-2", "endSession"),
        ])
    );
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>false<"), "{}", ends[0]);
    // The follow-ups name the new work item.
    for op in [
        "addCategory",
        "addLinkedItemWithRev",
        "addPlanItems",
        "setTestSteps",
    ] {
        let body = &bodies(&server, op).await[0];
        assert!(body.contains(&new_item_uri()), "{op}: {body}");
    }
    let pinned = &bodies(&server, "addLinkedItemWithRev").await[0];
    assert!(
        pinned.contains(">42<") && pinned.contains(">true<"),
        "{pinned}"
    );
}

#[tokio::test]
async fn creates_without_follow_ups_and_in_documents() {
    let server = session_server().await;
    answer_creates(&server).await;
    let polarion = writer(&server);
    let items = polarion.items();

    // Without follow-ups, the create runs on the client's session.
    items
        .create(&NewWorkItem::new("Sandbox", "task", "x"))
        .await
        .unwrap();
    // In a document, with a follow-up: in a session of its own.
    let new = NewWorkItem {
        categories: vec!["software".into()],
        ..NewWorkItem::new("Sandbox", "task", "y")
    };
    let document = DocumentRef::new("Sandbox", "Specs", "SRS");
    items
        .create_in_document(&new, &document, None)
        .await
        .unwrap();
    assert_eq!(
        log(&server).await,
        owned(&[
            ("", "logInWithToken"),
            ("-1", "getProject"),
            ("-1", "createWorkItem"),
            ("-1", "getProject"),
            ("", "logInWithToken"),
            ("-2", "beginTransaction"),
            ("-2", "createWorkItemInModule"),
            ("-2", "addCategory"),
            ("-2", "endTransaction"),
            ("-2", "endSession"),
        ])
    );
}

#[tokio::test]
async fn follow_ups_that_add_nothing_roll_the_create_back() {
    let server = session_server().await;
    answer_creates(&server).await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("addAssignee>"))
        .respond_with(xml(
            "<addAssigneeResponse><addAssigneeReturn>false</addAssigneeReturn></addAssigneeResponse>",
        ))
        .mount(&server)
        .await;
    let new = NewWorkItem {
        categories: vec!["software".into()],
        assignees: vec!["nobody".into()],
        approvees: vec!["boss".into()],
        ..NewWorkItem::new("Sandbox", "task", "x")
    };

    let err = writer(&server).items().create(&new).await.unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m.contains("addAssignee") && m.contains("nobody")),
        "{err:?}"
    );
    assert_eq!(
        operations(&log(&server).await),
        [
            "logInWithToken",
            "getProject",
            "logInWithToken",
            "beginTransaction",
            "createWorkItem",
            "addCategory",
            "addAssignee",
            "endTransaction",
            "endSession",
        ]
    );
    let ends = bodies(&server, "endTransaction").await;
    assert!(ends[0].contains("rollback>true<"), "{}", ends[0]);
}

#[tokio::test]
async fn document_test_run_and_plan_creates_write_their_fields_in_one_transaction() {
    let server = session_server().await;
    answer_creates(&server).await;
    let polarion = writer(&server);

    // Without fields: one create each, on the client's session.
    let document = NewDocument {
        project: "Sandbox".into(),
        space: Some("Specs".into()),
        name: "New".into(),
        structure_link_role: "parent".into(),
        ..NewDocument::default()
    };
    let run = NewTestRun {
        title: Some("Run".into()),
        ..NewTestRun::new("Sandbox", "R9", "tpl")
    };
    let plan = NewPlan::new("Sandbox", "rel9", "Release 9", "tpl");
    assert_eq!(
        polarion.documents().create(&document).await.unwrap(),
        new_document()
    );
    assert_eq!(polarion.tests().create(&run).await.unwrap(), new_run_uri());
    polarion.plans().create(&plan).await.unwrap();
    let plain = log(&server).await;
    assert_eq!(
        plain,
        owned(&[
            ("", "logInWithToken"),
            ("-1", "createDocument"),
            ("-1", "createTestRunWithTitle"),
            ("-1", "createPlan"),
        ])
    );

    // With fields: the create and its update in a session of their own.
    let document = NewDocument {
        kind: Some("specification".into()),
        status: Some("draft".into()),
        ..document
    };
    let run = NewTestRun {
        title: None,
        fields: TestRunUpdate {
            status: Some("open".into()),
            ..TestRunUpdate::default()
        },
        ..run
    };
    let plan = NewPlan {
        fields: PlanUpdate {
            due_date: Some("2024-12-31".into()),
            ..PlanUpdate::default()
        },
        ..plan
    };
    polarion.documents().create(&document).await.unwrap();
    polarion.tests().create(&run).await.unwrap();
    polarion.plans().create(&plan).await.unwrap();
    let log = log(&server).await;
    let mut sessions: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (s, o) in &log[plain.len()..] {
        sessions.entry(s.as_str()).or_default().push(o.as_str());
    }
    for (session, create, update) in [
        ("-2", "createDocument", "updateModule"),
        ("-3", "createTestRun", "updateTestRun"),
        ("-4", "createPlan", "updatePlan"),
    ] {
        assert_eq!(
            sessions[session],
            [
                "beginTransaction",
                create,
                update,
                "endTransaction",
                "endSession"
            ],
            "{log:?}"
        );
    }
    let module = &bodies(&server, "updateModule").await[0];
    assert!(
        module.contains(&new_document().uri) && module.contains(">draft<"),
        "{module}"
    );
    let updated = &bodies(&server, "updateTestRun").await[0];
    assert!(updated.contains(&new_run_uri()), "{updated}");

    // The title of a new test run and the name of a new plan are not fields.
    let err = polarion
        .tests()
        .create(&NewTestRun {
            fields: TestRunUpdate {
                title: Some("t".into()),
                ..TestRunUpdate::default()
            },
            ..NewTestRun::new("Sandbox", "R8", "tpl")
        })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = polarion
        .plans()
        .create(&NewPlan {
            fields: PlanUpdate {
                name: Some("n".into()),
                ..PlanUpdate::default()
            },
            ..NewPlan::new("Sandbox", "rel8", "R8", "tpl")
        })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert_eq!(log.len(), server.received_requests().await.unwrap().len());
}
