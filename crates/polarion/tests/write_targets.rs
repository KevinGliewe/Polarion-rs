//! Write targets (restricted mode): writes to the targets are sent; every other write is refused
//! before any request, by the library's guard and by the SOAP write filter (which also covers
//! `Polarion::soap()`); every SOAP write operation is classified.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use polarion::soap::types::{AnyValue, CustomField, EnumOptionId, WorkItem as SoapWorkItem};
use polarion::uri::test_run_uri;
use polarion::{
    Auth, CreateBudget, DocumentRef, DocumentUpdate, Error, FieldValue, ItemRef, NewDocument,
    NewWorkItem, NewWorkRecord, PlanUpdate, Polarion, ReuseDocument, TestRecord, TestRunUpdate,
    Text, UserUpdate, WorkItemUpdate,
};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const P: &str = "Sandbox";

fn item(id: &str) -> ItemRef {
    ItemRef::new(P, id)
}

/// The work item the client may write to.
fn a() -> ItemRef {
    item("SB-1")
}

/// A second target: link target, test case, plan item.
fn a_link() -> ItemRef {
    item("SB-2")
}

/// Not a target, in the same project.
fn b() -> ItemRef {
    item("SB-3")
}

/// Not a target: `SB-1` is a prefix of its id.
fn a2() -> ItemRef {
    item("SB-12")
}

/// A comment of the work item `uri`, in the format Polarion uses (`…${Comment}{WorkItem}ID#KEY`).
fn comment_of(uri: &str) -> String {
    format!(
        "{}#1",
        uri.replacen("${WorkItem}", "${Comment}{WorkItem}", 1)
    )
}

/// A work record of the work item `uri`, in the format Polarion uses
/// (`…${WorkRecord}{WorkItem}ID#KEY`).
fn record_of(uri: &str) -> String {
    format!(
        "{}#1",
        uri.replacen("${WorkItem}", "${WorkRecord}{WorkItem}", 1)
    )
}

/// A comment of the document `uri`, in the format Polarion uses
/// (`…${ModuleComment}{Module}{moduleFolder}SPACE#NAME#KEY`).
fn doc_comment_of(uri: &str) -> String {
    format!(
        "{}#1",
        uri.replacen("${Module}", "${ModuleComment}{Module}", 1)
    )
}

fn doc() -> DocumentRef {
    DocumentRef::new(P, "Specs", "SRS")
}

fn other_doc() -> DocumentRef {
    DocumentRef::new(P, "Specs", "SRS 2")
}

fn run() -> String {
    test_run_uri(P, "R1")
}

fn other_run() -> String {
    test_run_uri(P, "R2")
}

fn plan() -> String {
    format!("{URI}{P}${{Plan}}rel1")
}

fn other_plan() -> String {
    format!("{URI}{P}${{Plan}}rel2")
}

fn baseline() -> String {
    format!("{URI}{P}${{Baseline}}b1")
}

fn text(s: &str) -> Text {
    Text {
        content_type: "text/plain".into(),
        content: s.into(),
    }
}

fn builder(server: &MockServer) -> polarion::PolarionBuilder {
    Polarion::builder(&base_url(server), Auth::Token("t".into()))
}

/// A client restricted to the targets. The project and global allowlists must not widen it.
fn restricted(server: &MockServer) -> Polarion {
    builder(server)
        .allow_writes([P])
        .allow_global_writes(true)
        .restrict_writes([
            a().uri,
            // A revision suffix is ignored.
            format!("{}%1234", a_link().uri),
            doc().uri,
            run(),
            plan(),
            baseline(),
        ])
        .build()
        .unwrap()
}

/// The operation of a SOAP request (`addHyperlink`).
fn operation(body: &str) -> String {
    let rest = body
        .split_once("<soapenv:Body><")
        .map_or("", |(_, rest)| rest);
    let name = &rest[..rest.find(['>', ' ', '/']).unwrap_or(rest.len())];
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

/// Answers every operation: reads of the work item with a description, `getProject` with the
/// project, everything else with `true` (which decodes as `true`, a string, or nothing).
struct Anything;

impl Respond for Anything {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        let op = operation(&body);
        let inner = match op.as_str() {
            "getWorkItemByIdsWithFields" => format!(
                r#"<{op}Return uri="{}"><id>SB-1</id><description><type>text/html</type><content>&lt;p&gt;old&lt;/p&gt;</content></description></{op}Return>"#,
                a().uri
            ),
            "getProject" => format!(
                r#"<{op}Return uri="{URI}{P}${{Project}}{P}"><id>{P}</id><name>{P}</name></{op}Return>"#
            ),
            // Documents hold no work items.
            "getModuleWorkItemUris" => String::new(),
            _ => format!("<{op}Return>true</{op}Return>"),
        };
        xml(&format!("<{op}Response>{inner}</{op}Response>"))
    }
}

async fn answer_everything(server: &MockServer) {
    // Below the login mock of `server()`.
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Anything)
        .with_priority(10)
        .mount(server)
        .await;
}

/// The operations of the requests received, logins excluded, in order.
async fn sent(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| operation(&String::from_utf8_lossy(&r.body)))
        .filter(|op| op != "logInWithToken")
        .collect()
}

/// The fields of a `WriteNotAllowed`: `(project, target, reason)`.
type Refusal = (Option<String>, Option<String>, Option<&'static str>);

#[track_caller]
fn refusal<T: std::fmt::Debug>(result: polarion::Result<T>, call: &str) -> Refusal {
    match result {
        Err(Error::WriteNotAllowed {
            project,
            target,
            reason,
            ..
        }) => (project, target, reason),
        other => panic!("{call}: expected WriteNotAllowed, got {other:?}"),
    }
}

/// Expect the call to be refused with `WriteNotAllowed`; gives `(project, target, reason)`.
macro_rules! refused {
    ($call:expr) => {
        refusal($call.await, stringify!($call))
    };
}

/// Expect the refusal to name `target` as the object that is not a write target.
#[track_caller]
fn not_a_target(refusal: Refusal, target: &str) {
    assert_eq!(refusal.1.as_deref(), Some(target), "{refusal:?}");
    assert_eq!(refusal.2, None, "{refusal:?}");
}

/// Expect the refusal to give a reason (unbounded writes, status, creates).
#[track_caller]
fn with_reason(refusal: Refusal) {
    assert!(refusal.2.is_some(), "{refusal:?}");
}

#[tokio::test]
async fn bounded_writes_to_the_targets_are_sent() {
    let server = server().await;
    answer_everything(&server).await;
    let polarion = restricted(&server);
    let (items, docs, tests, plans) = (
        polarion.items(),
        polarion.documents(),
        polarion.tests(),
        polarion.plans(),
    );
    let note = text("note");
    let comment = comment_of(&a().uri);

    // Work items.
    let update = WorkItemUpdate {
        title: Some("New title".into()),
        description: Some(text("d")),
        custom_fields: BTreeMap::from([
            ("risk".to_owned(), Some(FieldValue::Int(3))),
            ("old".to_owned(), None),
        ]),
        clear: vec!["dueDate".into()],
        ..WorkItemUpdate::default()
    };
    items.update(&a(), &update).await.unwrap();
    items
        .set_custom_field(&a(), "risk", Some(&FieldValue::Int(4)))
        .await
        .unwrap();
    items.set_custom_field(&a(), "risk", None).await.unwrap();
    items.clear_fields(&a(), &["dueDate".into()]).await.unwrap();
    items.add_comment(&a(), Some("t"), &note).await.unwrap();
    items
        .add_restricted_comment(&a(), "t", &note, &["admin".into()])
        .await
        .unwrap();
    items.reply_to_comment(&a(), &comment, &note).await.unwrap();
    items
        .set_comment_resolved(&a(), &comment, true)
        .await
        .unwrap();
    items
        .set_comment_tags(&a(), &comment, &["tag"])
        .await
        .unwrap();
    items.add_link(&a(), "relates_to", &a_link()).await.unwrap();
    items
        .remove_link(&a(), "relates_to", &a_link())
        .await
        .unwrap();
    items
        .add_link_at_revision(&a(), "relates_to", &a_link(), Some("12"), false)
        .await
        .unwrap();
    let url = "https://example.invalid/polarion-rs";
    items.add_hyperlink(&a(), url, "ref_ext").await.unwrap();
    items.remove_hyperlink(&a(), url).await.unwrap();
    items
        .add_planning_constraint(&a(), "2024-01-31", "startNoEarlierThan")
        .await
        .unwrap();
    items
        .remove_planning_constraint(&a(), "2024-01-31", "startNoEarlierThan")
        .await
        .unwrap();
    items.add_assignee(&a(), "me").await.unwrap();
    items.remove_assignee(&a(), "me").await.unwrap();
    items.add_category(&a(), "c").await.unwrap();
    items.remove_category(&a(), "c").await.unwrap();
    items.add_approvee(&a(), "me").await.unwrap();
    items.set_approval(&a(), "me", "waiting").await.unwrap();
    items.remove_approvee(&a(), "me").await.unwrap();
    items
        .add_attachment(&a(), "a.txt", "A", b"x")
        .await
        .unwrap();
    items
        .update_attachment(&a(), "1-a.txt", None, Some("B"), None)
        .await
        .unwrap();
    items.delete_attachment(&a(), "1-a.txt").await.unwrap();
    let record = NewWorkRecord {
        user: "me".into(),
        date: "2024-01-31".into(),
        time_spent: "1h".into(),
        ..NewWorkRecord::default()
    };
    items.add_work_record(&a(), &record).await.unwrap();
    let typed = NewWorkRecord {
        kind: Some("dev".into()),
        comment: Some("c".into()),
        ..record
    };
    items.add_work_record(&a(), &typed).await.unwrap();
    items
        .delete_work_record(&a(), &record_of(&a().uri))
        .await
        .unwrap();
    items
        .set_localized_title(&a(), Some("de"), Some("Titel"))
        .await
        .unwrap();
    items
        .set_localized_description(&a(), None, Some(&note))
        .await
        .unwrap();
    #[cfg(feature = "markdown")]
    {
        let options = polarion::MarkdownWriteOptions::default();
        let set = items
            .set_description_markdown(&a(), "New *text*", &options)
            .await
            .unwrap();
        assert!(set.updated);
        items
            .append_description_markdown(&a(), "More", &options)
            .await
            .unwrap();
    }

    // Documents.
    let doc_update = DocumentUpdate {
        title: Some("New".into()),
        ..DocumentUpdate::default()
    };
    docs.update(&doc(), &doc_update).await.unwrap();
    // The body of a document without work items (the guard reads them first).
    let body_update = DocumentUpdate {
        body: Some(Text {
            content_type: "text/html".into(),
            content: "<p>New body</p>".into(),
        }),
        uses_outline_numbering: Some(true),
        ..DocumentUpdate::default()
    };
    docs.update(&doc(), &body_update).await.unwrap();
    docs.add_comment(&doc(), None, &note).await.unwrap();
    docs.reply_to_comment(&doc(), &doc_comment_of(&doc().uri), &note)
        .await
        .unwrap();

    // Test runs.
    let run_update = TestRunUpdate {
        title: Some("Nightly".into()),
        summary_defect: Some(a_link()),
        ..TestRunUpdate::default()
    };
    tests.update(&run(), &run_update).await.unwrap();
    tests.set_report(&run(), &note).await.unwrap();
    let passed = TestRecord {
        test_case: Some(a_link()),
        result: Some("passed".into()),
        ..TestRecord::default()
    };
    tests.add_record(&run(), &passed).await.unwrap();
    tests.update_record(&run(), 0, &passed).await.unwrap();
    tests.remove_record(&run(), 0).await.unwrap();
    tests
        .set_steps(&a_link(), &[vec![text("Open"), text("It opens")]])
        .await
        .unwrap();
    tests
        .add_attachment(&run(), "r.txt", "R", b"x")
        .await
        .unwrap();
    tests
        .update_attachment(&run(), "r.txt", Some("S"), None)
        .await
        .unwrap();
    tests.delete_attachment(&run(), "r.txt").await.unwrap();
    tests
        .add_record_attachment(&run(), 0, "r.txt", "R", b"x")
        .await
        .unwrap();
    tests
        .delete_record_attachment(&run(), 0, "r.txt")
        .await
        .unwrap();
    tests
        .add_step_attachment(&run(), 0, 0, "s.txt", "S", b"x")
        .await
        .unwrap();
    tests
        .delete_step_attachment(&run(), 0, 0, "s.txt")
        .await
        .unwrap();

    // Plans and baselines.
    let plan_update = PlanUpdate {
        name: Some("Release 1.1".into()),
        ..PlanUpdate::default()
    };
    plans.update(&plan(), &plan_update).await.unwrap();
    plans.set_report(&plan(), &note).await.unwrap();
    plans.add_items(&plan(), &[a(), a_link()]).await.unwrap();
    plans.remove_items(&plan(), &[a()]).await.unwrap();
    plans.add_allowed_type(&plan(), "task").await.unwrap();
    plans.remove_allowed_type(&plan(), "task").await.unwrap();
    polarion
        .baselines()
        .update(&baseline(), Some("B1"), None)
        .await
        .unwrap();

    // Transactions.
    polarion.begin_transaction().await.unwrap();
    polarion.rollback().await.unwrap();
    polarion.begin_transaction().await.unwrap();
    polarion.commit().await.unwrap();

    // Every operation the filter lets through as a write to a target (or a transaction) was
    // sent by a library method, so the filter accepts the requests the library builds.
    let sent: BTreeSet<String> = sent(&server).await.into_iter().collect();
    let expected: BTreeSet<String> = library_classes()
        .into_iter()
        .filter(|(_, class)| ["Target", "Owned", "Transaction"].contains(&class.as_str()))
        .map(|(op, _)| op)
        .collect();
    let writes: BTreeSet<String> = sent
        .iter()
        .filter(|op| !op.starts_with("get"))
        .cloned()
        .collect();
    assert_eq!(writes, expected);
    assert!(sent.contains("getModuleWorkItemUris"), "{sent:?}");
}

#[tokio::test]
async fn writes_to_anything_else_are_refused_before_any_request() {
    let server = server().await;
    let polarion = restricted(&server);
    let (items, docs, tests, plans) = (
        polarion.items(),
        polarion.documents(),
        polarion.tests(),
        polarion.plans(),
    );
    let note = text("note");
    let title = WorkItemUpdate {
        title: Some("x".into()),
        ..WorkItemUpdate::default()
    };
    let b_uri = b().uri;

    // The bounded writes, on B (same project, not a target).
    not_a_target(refused!(items.update(&b(), &title)), &b_uri);
    not_a_target(
        refused!(items.set_custom_field(&b(), "risk", Some(&FieldValue::Int(1)))),
        &b_uri,
    );
    not_a_target(
        refused!(items.clear_fields(&b(), &["dueDate".into()])),
        &b_uri,
    );
    not_a_target(refused!(items.add_comment(&b(), None, &note)), &b_uri);
    not_a_target(
        refused!(items.add_restricted_comment(&b(), "t", &note, &[])),
        &b_uri,
    );
    not_a_target(
        refused!(items.add_hyperlink(&b(), "https://example.invalid", "ref_ext")),
        &b_uri,
    );
    // The canary of the live suite: harmless even if it got through.
    not_a_target(
        refused!(items.remove_hyperlink(&b(), "https://example.invalid/polarion-rs-canary")),
        &b_uri,
    );
    not_a_target(refused!(items.add_category(&b(), "c")), &b_uri);
    not_a_target(refused!(items.remove_category(&b(), "c")), &b_uri);
    not_a_target(
        refused!(items.add_planning_constraint(&b(), "2024-01-31", "x")),
        &b_uri,
    );
    not_a_target(
        refused!(items.remove_planning_constraint(&b(), "2024-01-31", "x")),
        &b_uri,
    );
    not_a_target(refused!(items.add_assignee(&b(), "me")), &b_uri);
    not_a_target(refused!(items.remove_assignee(&b(), "me")), &b_uri);
    not_a_target(refused!(items.add_approvee(&b(), "me")), &b_uri);
    not_a_target(refused!(items.remove_approvee(&b(), "me")), &b_uri);
    not_a_target(refused!(items.set_approval(&b(), "me", "waiting")), &b_uri);
    not_a_target(refused!(items.add_attachment(&b(), "a", "A", b"x")), &b_uri);
    not_a_target(
        refused!(items.update_attachment(&b(), "1-a", None, Some("t"), None)),
        &b_uri,
    );
    not_a_target(refused!(items.delete_attachment(&b(), "1-a")), &b_uri);
    let record = NewWorkRecord {
        user: "me".into(),
        date: "2024-01-31".into(),
        time_spent: "1h".into(),
        ..NewWorkRecord::default()
    };
    not_a_target(refused!(items.add_work_record(&b(), &record)), &b_uri);
    not_a_target(
        refused!(items.set_localized_title(&b(), None, Some("t"))),
        &b_uri,
    );
    not_a_target(
        refused!(items.set_localized_description(&b(), None, Some(&note))),
        &b_uri,
    );
    #[cfg(feature = "markdown")]
    {
        let options = polarion::MarkdownWriteOptions::default();
        not_a_target(
            refused!(items.set_description_markdown(&b(), "x", &options)),
            &b_uri,
        );
        not_a_target(
            refused!(items.append_description_markdown(&b(), "x", &options)),
            &b_uri,
        );
    }

    // Links A→C, and B→A.
    let c = item("SB-4");
    not_a_target(refused!(items.add_link(&a(), "relates_to", &c)), &c.uri);
    not_a_target(refused!(items.remove_link(&a(), "relates_to", &c)), &c.uri);
    not_a_target(
        refused!(items.add_link_at_revision(&a(), "relates_to", &c, None, false)),
        &c.uri,
    );
    not_a_target(refused!(items.add_link(&b(), "relates_to", &a())), &b_uri);

    // Comments and work records of other work items, `…SB-12` included although the target is
    // `…SB-1`, and URIs in unknown formats.
    let trap = comment_of(&a2().uri);
    not_a_target(refused!(items.reply_to_comment(&a(), &trap, &note)), &trap);
    not_a_target(
        refused!(items.set_comment_resolved(&a(), &trap, true)),
        &trap,
    );
    not_a_target(refused!(items.set_comment_tags(&a(), &trap, &["t"])), &trap);
    not_a_target(refused!(items.delete_work_record(&a(), &trap)), &trap);
    let b_comment = comment_of(&b_uri);
    not_a_target(
        refused!(items.reply_to_comment(&a(), &b_comment, &note)),
        &b_comment,
    );
    for unknown in ["c-1", "#1", &format!("{}1/1", a().uri)] {
        not_a_target(
            refused!(items.reply_to_comment(&a(), unknown, &note)),
            unknown,
        );
    }
    let a_comment = comment_of(&a().uri);
    not_a_target(
        refused!(items.reply_to_comment(&b(), &a_comment, &note)),
        &b_uri,
    );
    // A comment of another target is not the given work item's.
    let a_link_comment = comment_of(&a_link().uri);
    not_a_target(
        refused!(items.reply_to_comment(&a(), &a_link_comment, &note)),
        &a_link_comment,
    );
    // The key after the owner names nothing else: no second object, no path.
    for trap in [
        format!("{}/x${{WorkItem}}SB-3", a().uri),
        format!("{}#wr${{WorkItem}}SB-3", a().uri),
        format!("{}/1/../../SB-3", a().uri),
        format!("{}/%7Bx%7D", a().uri),
    ] {
        not_a_target(
            refused!(items.set_comment_resolved(&a(), &trap, true)),
            &trap,
        );
        not_a_target(refused!(items.delete_work_record(&a(), &trap)), &trap);
    }
    let doc_trap = format!("{}#c1${{WorkItem}}SB-3", doc().uri);
    not_a_target(
        refused!(docs.reply_to_comment(&doc(), &doc_trap, &note)),
        &doc_trap,
    );

    // Documents, test runs, plans, baselines that are not targets, and further objects.
    let doc_update = DocumentUpdate {
        title: Some("x".into()),
        ..DocumentUpdate::default()
    };
    not_a_target(
        refused!(docs.update(&other_doc(), &doc_update)),
        &other_doc().uri,
    );
    not_a_target(
        refused!(docs.add_comment(&other_doc(), None, &note)),
        &other_doc().uri,
    );
    let other_doc_comment = doc_comment_of(&other_doc().uri);
    not_a_target(
        refused!(docs.reply_to_comment(&doc(), &other_doc_comment, &note)),
        &other_doc_comment,
    );
    let run_title = TestRunUpdate {
        title: Some("x".into()),
        ..TestRunUpdate::default()
    };
    not_a_target(
        refused!(tests.update(&other_run(), &run_title)),
        &other_run(),
    );
    let with_defect = TestRunUpdate {
        summary_defect: Some(b()),
        ..TestRunUpdate::default()
    };
    not_a_target(refused!(tests.update(&run(), &with_defect)), &b_uri);
    not_a_target(
        refused!(tests.set_report(&other_run(), &note)),
        &other_run(),
    );
    let of_b = TestRecord {
        test_case: Some(b()),
        result: Some("passed".into()),
        ..TestRecord::default()
    };
    not_a_target(refused!(tests.add_record(&run(), &of_b)), &b_uri);
    not_a_target(refused!(tests.update_record(&run(), 0, &of_b)), &b_uri);
    let defect = TestRecord {
        test_case: Some(a_link()),
        defect: Some(b()),
        ..TestRecord::default()
    };
    not_a_target(refused!(tests.add_record(&run(), &defect)), &b_uri);
    let executed_by = TestRecord {
        test_case: Some(a_link()),
        executed_by: Some("amy".into()),
        ..TestRecord::default()
    };
    not_a_target(
        refused!(tests.add_record(&run(), &executed_by)),
        &polarion::uri::user_uri("amy"),
    );
    not_a_target(refused!(tests.remove_record(&other_run(), 0)), &other_run());
    not_a_target(refused!(tests.set_steps(&b(), &[])), &b_uri);
    not_a_target(
        refused!(tests.add_attachment(&other_run(), "f", "F", b"x")),
        &other_run(),
    );
    not_a_target(
        refused!(tests.update_attachment(&other_run(), "f", None, None)),
        &other_run(),
    );
    not_a_target(
        refused!(tests.delete_attachment(&other_run(), "f")),
        &other_run(),
    );
    not_a_target(
        refused!(tests.add_record_attachment(&other_run(), 0, "f", "F", b"x")),
        &other_run(),
    );
    not_a_target(
        refused!(tests.delete_record_attachment(&other_run(), 0, "f")),
        &other_run(),
    );
    not_a_target(
        refused!(tests.add_step_attachment(&other_run(), 0, 0, "f", "F", b"x")),
        &other_run(),
    );
    not_a_target(
        refused!(tests.delete_step_attachment(&other_run(), 0, 0, "f")),
        &other_run(),
    );
    let plan_name = PlanUpdate {
        name: Some("x".into()),
        ..PlanUpdate::default()
    };
    not_a_target(
        refused!(plans.update(&other_plan(), &plan_name)),
        &other_plan(),
    );
    not_a_target(
        refused!(plans.set_report(&other_plan(), &note)),
        &other_plan(),
    );
    not_a_target(refused!(plans.add_items(&plan(), &[a(), b()])), &b_uri);
    not_a_target(refused!(plans.remove_items(&plan(), &[b()])), &b_uri);
    not_a_target(
        refused!(plans.add_allowed_type(&other_plan(), "task")),
        &other_plan(),
    );
    not_a_target(
        refused!(plans.remove_allowed_type(&other_plan(), "task")),
        &other_plan(),
    );
    let other_baseline = format!("{URI}{P}${{Baseline}}b2");
    not_a_target(
        refused!(
            polarion
                .baselines()
                .update(&other_baseline, Some("x"), None)
        ),
        &other_baseline,
    );

    // `status` and `resolution`, on the targets.
    let resolution = WorkItemUpdate {
        resolution: Some("done".into()),
        ..WorkItemUpdate::default()
    };
    with_reason(refused!(items.update(&a(), &resolution)));
    let status_field = WorkItemUpdate {
        custom_fields: BTreeMap::from([(
            "status".to_owned(),
            Some(FieldValue::Enum("done".into())),
        )]),
        ..WorkItemUpdate::default()
    };
    with_reason(refused!(items.update(&a(), &status_field)));
    let cleared = WorkItemUpdate {
        clear: vec!["Status".into()],
        ..WorkItemUpdate::default()
    };
    with_reason(refused!(items.update(&a(), &cleared)));
    with_reason(refused!(items.set_custom_field(
        &a(),
        "status",
        Some(&FieldValue::Enum("done".into()))
    )));
    with_reason(refused!(items.set_custom_field(&a(), "resolution", None)));
    with_reason(refused!(items.clear_fields(&a(), &["resolution".into()])));
    let doc_status = DocumentUpdate {
        status: Some("published".into()),
        ..DocumentUpdate::default()
    };
    with_reason(refused!(docs.update(&doc(), &doc_status)));
    let run_status = TestRunUpdate {
        status: Some("finished".into()),
        ..TestRunUpdate::default()
    };
    with_reason(refused!(tests.update(&run(), &run_status)));
    let plan_status = PlanUpdate {
        status: Some("done".into()),
        ..PlanUpdate::default()
    };
    with_reason(refused!(plans.update(&plan(), &plan_status)));

    // Unbounded writes: refused even on the targets.
    let new_item = NewWorkItem::new(P, "task", "x");
    with_reason(refused!(items.delete(&a())));
    with_reason(refused!(items.move_to_document(
        &a(),
        &doc(),
        None,
        None,
        false
    )));
    with_reason(refused!(items.perform_action(&a(), 1)));
    with_reason(refused!(items.transition(&a(), "done")));
    with_reason(refused!(items.reset_workflow(&a())));
    with_reason(refused!(items.auto_suspect(&a())));
    with_reason(refused!(items.auto_assign(&a())));
    with_reason(refused!(items.add_external_link(&a(), "https://x", "r")));
    with_reason(refused!(items.remove_external_link(&a(), "https://x", "r")));
    with_reason(refused!(items.add_linked_revision(&a(), None, "1")));
    with_reason(refused!(items.remove_linked_revision(
        &a(),
        Some("svn"),
        "1"
    )));
    with_reason(refused!(items.add_oslc_link(&a(), "https://x", "r", "l")));
    with_reason(refused!(items.create_in_document(&new_item, &doc(), None)));
    let new_doc = NewDocument {
        project: P.into(),
        name: "X".into(),
        structure_link_role: "parent".into(),
        ..NewDocument::default()
    };
    let reuse = ReuseDocument {
        project: P.into(),
        name: "Copy".into(),
        ..ReuseDocument::default()
    };
    with_reason(refused!(docs.delete(&doc())));
    with_reason(refused!(docs.create_module(&new_doc, true)));
    with_reason(refused!(docs.update_title_heading(&doc(), Some("t"))));
    with_reason(refused!(docs.reuse(&doc(), &reuse)));
    with_reason(refused!(docs.reuse_module(&doc(), &reuse, &[], &[])));
    with_reason(refused!(docs.update_derived(&doc(), None, false)));
    with_reason(refused!(docs.update_derived_module(&doc(), None)));
    with_reason(refused!(docs.add_comment(&doc(), Some(&a()), &note)));
    with_reason(refused!(docs.perform_action(&doc(), 1)));
    with_reason(refused!(docs.transition(&doc(), "published")));
    with_reason(refused!(tests.execute(&run(), &[])));
    with_reason(refused!(tests.create_summary_defect(&run(), "tpl")));
    with_reason(refused!(tests.update_summary_defect(
        &run(),
        "s",
        1,
        0,
        1,
        "tpl"
    )));
    with_reason(refused!(tests.add_project_to_span(&run(), "Other")));
    with_reason(refused!(tests.remove_project_from_span(&run(), "Other")));
    with_reason(refused!(plans.create(P, "p", "P", None, "tpl")));
    with_reason(refused!(plans.create_template(P, "p", "P", None)));
    with_reason(refused!(plans.delete(P, &["rel1".into()])));
    with_reason(refused!(plans.add_project_to_span(&plan(), "Other")));
    with_reason(refused!(plans.remove_project_from_span(&plan(), "Other")));
    with_reason(refused!(polarion.baselines().create(P, "B", None, None)));
    with_reason(refused!(polarion.baselines().delete(&baseline())));
    with_reason(refused!(polarion.folders().create(Some(P), None, "f", "F")));
    with_reason(refused!(polarion.folders().create(None, None, "f", "F")));
    with_reason(refused!(polarion.users().create("jdoe")));
    with_reason(refused!(
        polarion.users().update("jdoe", &UserUpdate::default())
    ));
    with_reason(refused!(
        polarion.security().update_user_account_vault("k", "u", "p")
    ));
    with_reason(refused!(polarion.activities().add_comment("a-1", &note)));

    // Creates without a budget.
    with_reason(refused!(items.create(&new_item)));
    with_reason(refused!(docs.create(&new_doc)));
    with_reason(refused!(tests.create(P, "R9", "tpl")));
    with_reason(refused!(tests.create_with_title(P, "R9", None, "tpl")));

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test]
async fn restricted_mode_without_targets_writes_nothing() {
    let server = server().await;
    let polarion = builder(&server)
        .allow_writes([P])
        .allow_global_writes(true)
        .restrict_writes(Vec::<String>::new())
        .build()
        .unwrap();
    assert!(polarion.writes_restricted());
    assert!(polarion.soap().is_read_only());
    assert_eq!(polarion.writable_projects().count(), 0);
    assert!(!polarion.global_writes_allowed());
    assert!(!polarion.write_allowed(&a().uri));
    not_a_target(
        refused!(polarion.items().add_hyperlink(&a(), "https://x", "r")),
        &a().uri,
    );
    with_reason(refused!(polarion.begin_transaction()));
    with_reason(refused!(polarion.users().create("jdoe")));
    let err = polarion
        .soap()
        .tracker()
        .delete_work_item(&a().uri)
        .await
        .unwrap_err();
    assert!(
        matches!(err, polarion::soap::Error::ReadOnly { .. }),
        "{err:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn creates_stay_within_the_budget_of_each_kind() {
    let server = server().await;
    answer_everything(&server).await;
    let budget = CreateBudget {
        work_items: 1,
        test_runs: 1,
        documents: 1,
    };
    let polarion = builder(&server)
        .allow_writes(["Prod"])
        .allow_creates(P, budget)
        .build()
        .unwrap();
    assert!(polarion.writes_restricted() && !polarion.soap().is_read_only());
    let (items, docs, tests) = (polarion.items(), polarion.documents(), polarion.tests());
    let new_doc = NewDocument {
        project: P.into(),
        name: "X".into(),
        structure_link_role: "parent".into(),
        ..NewDocument::default()
    };

    // Other projects never, also with a budget left.
    let (project, _, reason) = refused!(items.create(&NewWorkItem::new("Prod", "task", "x")));
    assert_eq!((project.as_deref(), reason.is_some()), (Some("Prod"), true));
    with_reason(refused!(tests.create("Prod", "R1", "tpl")));
    with_reason(refused!(docs.create(&NewDocument {
        project: "Prod".into(),
        ..new_doc.clone()
    })));
    // A status is never written, and the refusal takes nothing from the budget.
    let mut with_status = NewWorkItem::new(P, "task", "x");
    with_status
        .custom_fields
        .insert("status".into(), FieldValue::Enum("done".into()));
    with_reason(refused!(items.create(&with_status)));
    assert!(sent(&server).await.is_empty());

    items
        .create(&NewWorkItem::new(P, "task", "x"))
        .await
        .unwrap();
    tests.create(P, "R1", "tpl").await.unwrap();
    docs.create(&new_doc).await.unwrap();
    assert_eq!(
        sent(&server).await,
        [
            "getProject",
            "createWorkItem",
            "createTestRun",
            "createDocument"
        ]
    );

    // The second of each kind.
    with_reason(refused!(items.create(&NewWorkItem::new(P, "task", "y"))));
    with_reason(refused!(tests.create_with_title(P, "R2", Some("t"), "tpl")));
    with_reason(refused!(docs.create(&new_doc)));
    // Without targets, existing objects stay untouched.
    not_a_target(
        refused!(items.add_hyperlink(&a(), "https://x", "r")),
        &a().uri,
    );
    assert_eq!(sent(&server).await.len(), 4);

    // The filter keeps its own count: raw creates are refused once the budget is used up.
    let content = SoapWorkItem {
        project: Some(polarion::soap::types::Project {
            uri: Some(format!("{URI}{P}${{Project}}{P}")),
            ..Default::default()
        }),
        ..SoapWorkItem::default()
    };
    let err = polarion
        .soap()
        .tracker()
        .create_work_item(&content)
        .await
        .unwrap_err();
    assert!(
        matches!(err, polarion::soap::Error::WriteFiltered { .. }),
        "{err:?}"
    );
    assert_eq!(sent(&server).await.len(), 4);
}

#[tokio::test]
async fn every_create_attempt_takes_one_even_if_it_fails() {
    let server = server().await;
    Mock::given(wiremock::matchers::body_string_contains("getProject>"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::body_string_contains("hasSubject>"))
        .respond_with(xml(
            "<hasSubjectResponse><hasSubjectReturn>true</hasSubjectReturn></hasSubjectResponse>",
        ))
        .mount(&server)
        .await;
    let polarion = builder(&server)
        .allow_creates(
            P,
            CreateBudget {
                work_items: 1,
                ..CreateBudget::default()
            },
        )
        .build()
        .unwrap();
    let new_item = NewWorkItem::new(P, "task", "x");
    let err = polarion.items().create(&new_item).await.unwrap_err();
    assert!(matches!(err, Error::Soap(_)), "{err:?}");
    with_reason(refused!(polarion.items().create(&new_item)));
    assert!(!sent(&server).await.contains(&"createWorkItem".to_owned()));
}

#[tokio::test]
async fn write_allowed_answers_for_the_targets_only() {
    let server = server().await;
    let polarion = restricted(&server);
    for uri in [
        a().uri,
        format!("{}%99", a().uri),
        a_link().uri,
        doc().uri,
        run(),
        plan(),
        baseline(),
    ] {
        assert!(polarion.write_allowed(&uri), "{uri}");
    }
    for uri in [
        b().uri,
        a2().uri,
        comment_of(&a().uri),
        other_doc().uri,
        other_run(),
        "junk".to_owned(),
    ] {
        assert!(!polarion.write_allowed(&uri), "{uri}");
    }
    let targets: Vec<&str> = polarion.write_targets().collect();
    assert_eq!(targets.len(), 6);
    assert!(targets.contains(&a_link().uri.as_str()), "{targets:?}");
    assert_eq!(polarion.writable_projects().count(), 0);
    assert!(!polarion.global_writes_allowed());
    assert!(polarion.soap().has_write_filter());

    // Without restricted mode: the project allowlist.
    let writer = writer(&server);
    assert!(!writer.writes_restricted() && !writer.soap().has_write_filter());
    assert!(writer.write_allowed(&b().uri));
    assert!(!writer.write_allowed(&ItemRef::new("Prod", "PR-1").uri));
    assert!(!writer.write_allowed("junk"));
    assert!(!reader(&server).write_allowed(&a().uri));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn raw_soap_writes_are_filtered() {
    let server = server().await;
    answer_everything(&server).await;
    let polarion = restricted(&server);
    let tracker = polarion.soap().tracker();
    let role = EnumOptionId {
        id: Some("relates_to".into()),
    };
    let filtered = |result: polarion::soap::Result<_>, what: &str| match result {
        Err(polarion::soap::Error::WriteFiltered { .. }) => {}
        other => panic!("{what}: expected WriteFiltered, got {other:?}"),
    };

    filtered(
        tracker
            .add_hyperlink(&b().uri, "https://x", &role)
            .await
            .map(drop),
        "hyperlink on B",
    );
    filtered(
        tracker
            .add_linked_item(&a().uri, &b().uri, &role)
            .await
            .map(drop),
        "link A→B",
    );
    filtered(
        tracker
            .add_comment(&comment_of(&a2().uri), None, None)
            .await
            .map(drop),
        "comment of A2",
    );
    filtered(
        tracker.delete_work_item(&a().uri).await,
        "delete A (not a filtered operation)",
    );
    filtered(
        tracker.perform_workflow_action(&a().uri, 1).await,
        "workflow action on A",
    );
    filtered(
        tracker
            .set_custom_field(&CustomField {
                key: "status".into(),
                parent_item_uri: a().uri,
                value: None,
            })
            .await,
        "status of A",
    );
    filtered(
        tracker
            .update_work_item(&SoapWorkItem {
                uri: Some(a().uri),
                resolution: Some(EnumOptionId {
                    id: Some("done".into()),
                }),
                ..SoapWorkItem::default()
            })
            .await,
        "resolution of A",
    );
    filtered(
        tracker
            .create_work_item(&SoapWorkItem::default())
            .await
            .map(drop),
        "create without a budget",
    );
    filtered(
        tracker
            .create_document(P, None, "X", None, &[], &role, None)
            .await
            .map(drop),
        "document create without a budget",
    );
    filtered(
        polarion
            .soap()
            .project()
            .create_user("jdoe")
            .await
            .map(drop),
        "user",
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );

    // A raw write to a target goes through.
    assert!(
        tracker
            .add_hyperlink(&a().uri, "https://x", &role)
            .await
            .unwrap()
    );
    assert_eq!(sent(&server).await, ["addHyperlink"]);
}

/// A `WriteFiltered` refusal of a raw SOAP call.
#[track_caller]
fn filtered<T: std::fmt::Debug>(result: polarion::soap::Result<T>, what: &str) {
    match result {
        Err(polarion::soap::Error::WriteFiltered { .. }) => {}
        other => panic!("{what}: expected WriteFiltered, got {other:?}"),
    }
}

/// A raw custom field write to A whose value is `xml`, written verbatim.
fn raw_value(xml: &str) -> CustomField {
    CustomField {
        key: "risk".into(),
        parent_item_uri: a().uri,
        value: Some(AnyValue::Unknown {
            xsi_type: None,
            xml: xml.to_owned(),
        }),
    }
}

#[tokio::test]
async fn raw_xml_cannot_hide_or_add_objects() {
    let server = server().await;
    answer_everything(&server).await;
    let polarion = restricted(&server);
    let tracker = polarion.soap().tracker();
    let b_uri = b().uri;
    let hidden = b_uri.replacen("data-service", "data-<!-- -->service", 1);

    // Raw values that close the value and add fields, hide a URI in a comment or a processing
    // instruction, or are not XML at all.
    for (xml, what) in [
        (
            format!(
                "<trackert:value>1</trackert:value><trackert:parentItemURI>{b_uri}</trackert:parentItemURI>"
            ),
            "a second parent",
        ),
        (
            format!(
                "<trackert:value>1</trackert:value><trackert:parentItemURI>{hidden}</trackert:parentItemURI>"
            ),
            "a second parent hidden by a comment",
        ),
        (
            "<trackert:value>1</trackert:value><trackert:key>status</trackert:key>".to_owned(),
            "a second key",
        ),
        (
            format!("<trackert:value><?x {b_uri}?>1</trackert:value>"),
            "a processing instruction",
        ),
        (
            r#"<trackert:value xmlns:trackert="urn:other">1</trackert:value>"#.to_owned(),
            "another namespace",
        ),
        ("<trackert:value>1".to_owned(), "not XML"),
    ] {
        filtered(tracker.set_custom_field(&raw_value(&xml)).await, what);
    }
    // The target named in the wrong place: no URI on the content, B by project and id.
    let by_id = SoapWorkItem {
        id: Some("SB-3".into()),
        project: Some(polarion::soap::types::Project {
            id: Some(P.into()),
            ..Default::default()
        }),
        title: Some(a().uri),
        ..SoapWorkItem::default()
    };
    filtered(tracker.update_work_item(&by_id).await, "update B by id");
    let with_id = SoapWorkItem {
        uri: Some(a().uri),
        id: Some("SB-3".into()),
        ..SoapWorkItem::default()
    };
    filtered(
        tracker.update_work_item(&with_id).await,
        "update A naming SB-3",
    );
    let role = EnumOptionId {
        id: Some("relates_to".into()),
    };
    // A text URI goes up to the end of the text, quotes included.
    filtered(
        tracker
            .add_hyperlink(&format!("{}\"x", a().uri), "https://x", &role)
            .await
            .map(drop),
        "a URI with a quote after it",
    );
    filtered(
        tracker
            .add_comment(&format!("{}/x${{WorkItem}}SB-3", a().uri), None, None)
            .await
            .map(drop),
        "a comment key naming SB-3",
    );
    // A document's body, link settings and work items only through the library's checks.
    let module = |body: &str| polarion::soap::types::Module {
        uri: Some(doc().uri),
        home_page_content: Some(polarion::soap::types::Text {
            type_: "text/html".into(),
            content: body.into(),
            content_lossy: false,
        }),
        ..Default::default()
    };
    filtered(
        tracker.update_module(&module("<p>x</p>")).await,
        "a body nobody checked",
    );
    filtered(
        tracker
            .update_module(&module(
                r#"<div id="polarion_wiki macro name=module-workitem;params=id=SB-3"></div>"#,
            ))
            .await,
        "a body with a work item macro",
    );
    filtered(
        tracker
            .update_module(&polarion::soap::types::Module {
                uri: Some(doc().uri),
                structure_link_role: Some(role.clone()),
                ..Default::default()
            })
            .await,
        "the structure link role",
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test]
async fn raw_creates_name_their_project_where_it_belongs() {
    let server = server().await;
    answer_everything(&server).await;
    let polarion = builder(&server)
        .allow_creates(
            P,
            CreateBudget {
                work_items: 1,
                ..CreateBudget::default()
            },
        )
        .build()
        .unwrap();
    let tracker = polarion.soap().tracker();
    let project_uri = format!("{URI}{P}${{Project}}{P}");
    // Another project by id only, with the budget's project URI as the title.
    let by_id = SoapWorkItem {
        project: Some(polarion::soap::types::Project {
            id: Some("Other".into()),
            ..Default::default()
        }),
        title: Some(project_uri.clone()),
        ..SoapWorkItem::default()
    };
    filtered(
        tracker.create_work_item(&by_id).await.map(drop),
        "another project by id",
    );
    let mismatched = SoapWorkItem {
        project: Some(polarion::soap::types::Project {
            uri: Some(project_uri.clone()),
            id: Some("Other".into()),
            ..Default::default()
        }),
        ..SoapWorkItem::default()
    };
    filtered(
        tracker.create_work_item(&mismatched).await.map(drop),
        "the project's URI with another id",
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    // The budget is still there for a create in the project.
    let content = SoapWorkItem {
        project: Some(polarion::soap::types::Project {
            uri: Some(project_uri),
            id: Some(P.into()),
            ..Default::default()
        }),
        ..SoapWorkItem::default()
    };
    tracker.create_work_item(&content).await.unwrap();
    assert_eq!(sent(&server).await, ["createWorkItem"]);
}

#[tokio::test]
async fn a_create_is_not_repeated_beyond_the_budget() {
    let server = server().await;
    Mock::given(wiremock::matchers::body_string_contains("getProject>"))
        .respond_with(xml(&format!(
            r#"<getProjectResponse><getProjectReturn uri="{URI}{P}${{Project}}{P}"><id>{P}</id></getProjectReturn></getProjectResponse>"#
        )))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::body_string_contains("createWorkItem>"))
        .respond_with(ResponseTemplate::new(500).set_body_raw(
            envelope(
                "",
                "<soapenv:Fault><faultcode>soapenv:Server</faultcode><faultstring>Session expired</faultstring></soapenv:Fault>",
            ),
            "text/xml",
        ))
        .expect(1)
        .mount(&server)
        .await;
    // The session has expired: the client logs in again and would repeat the create.
    Mock::given(wiremock::matchers::body_string_contains("hasSubject>"))
        .respond_with(xml(
            "<hasSubjectResponse><hasSubjectReturn>false</hasSubjectReturn></hasSubjectResponse>",
        ))
        .mount(&server)
        .await;
    let polarion = builder(&server)
        .allow_creates(
            P,
            CreateBudget {
                work_items: 1,
                ..CreateBudget::default()
            },
        )
        .build()
        .unwrap();
    let err = polarion
        .items()
        .create(&NewWorkItem::new(P, "task", "x"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Soap(_)), "{err:?}");
    let sent = sent(&server).await;
    let creates = sent.iter().filter(|op| *op == "createWorkItem").count();
    assert_eq!(
        creates, 1,
        "at most the budget's number of create requests: {sent:?}"
    );
    assert_eq!(
        sent.iter().filter(|op| *op == "hasSubject").count(),
        1,
        "the session was renewed: {sent:?}"
    );
}

#[tokio::test]
async fn document_bodies_keep_the_work_items_of_documents() {
    let html = |content: &str| Text {
        content_type: "text/html".into(),
        content: content.into(),
    };
    let body = |content: &str| DocumentUpdate {
        body: Some(html(content)),
        ..DocumentUpdate::default()
    };

    // Refused before anything is sent: the structure link role, work item macros.
    let server = server().await;
    let polarion = restricted(&server);
    let docs = polarion.documents();
    let role = DocumentUpdate {
        structure_link_role: Some("parent".into()),
        ..DocumentUpdate::default()
    };
    with_reason(refused!(docs.update(&doc(), &role)));
    let macro_body =
        body(r#"<div id="polarion_wiki macro name=module-workitem;params=id=SB-3"></div>"#);
    with_reason(refused!(docs.update(&doc(), &macro_body)));
    let new_doc = NewDocument {
        project: P.into(),
        name: "X".into(),
        structure_link_role: "parent".into(),
        body: Some(
            r#"<div id="polarion_wiki macro name=Module-WorkItem;params=id=SB-3"></div>"#.into(),
        ),
        ..NewDocument::default()
    };
    let creator = builder(&server)
        .allow_creates(
            P,
            CreateBudget {
                documents: 1,
                ..CreateBudget::default()
            },
        )
        .build()
        .unwrap();
    with_reason(refused!(creator.documents().create(&new_doc)));
    assert!(server.received_requests().await.unwrap().is_empty());

    // A document that holds work items: its body and outline numbering are refused after the
    // read, its title is written.
    let server = common::server().await;
    Mock::given(wiremock::matchers::body_string_contains(
        "getModuleWorkItemUris>",
    ))
    .respond_with(xml(&format!(
        "<getModuleWorkItemUrisResponse><getModuleWorkItemUrisReturn>{}</getModuleWorkItemUrisReturn></getModuleWorkItemUrisResponse>",
        b().uri
    )))
    .mount(&server)
    .await;
    answer_everything(&server).await;
    let polarion = restricted(&server);
    let docs = polarion.documents();
    with_reason(refused!(docs.update(&doc(), &body("<p>x</p>"))));
    let numbering = DocumentUpdate {
        uses_outline_numbering: Some(false),
        ..DocumentUpdate::default()
    };
    with_reason(refused!(docs.update(&doc(), &numbering)));
    let title = DocumentUpdate {
        title: Some("t".into()),
        ..DocumentUpdate::default()
    };
    docs.update(&doc(), &title).await.unwrap();
    assert_eq!(
        sent(&server).await,
        [
            "getModuleWorkItemUris",
            "getModuleWorkItemUris",
            "updateModule"
        ]
    );
    // Its outline numbering through the raw client: the library did not check it.
    filtered(
        polarion
            .soap()
            .tracker()
            .update_module(&polarion::soap::types::Module {
                uri: Some(doc().uri),
                uses_outline_numbering: Some(false),
                ..Default::default()
            })
            .await,
        "outline numbering without the library's check",
    );
    assert_eq!(sent(&server).await.len(), 3);
}

// ---- the classification of every SOAP write operation

/// Operations a write-restricted client may send, with their class in `src/restricted.rs`.
const ALLOWED: &[(&str, &str)] = &[
    ("updateWorkItem", "Target"),
    ("setCustomField", "Target"),
    ("setFieldsNull", "Target"),
    ("addHyperlink", "Target"),
    ("removeHyperlink", "Target"),
    ("addCategory", "Target"),
    ("removeCategory", "Target"),
    ("addPlaningContraint", "Target"),
    ("removePlaningConstraint", "Target"),
    ("addAssignee", "Target"),
    ("removeAssignee", "Target"),
    ("addApprovee", "Target"),
    ("removeApprovee", "Target"),
    ("editApproval", "Target"),
    ("createAttachment", "Target"),
    ("updateAttachment", "Target"),
    ("deleteAttachment", "Target"),
    ("createWorkRecord", "Target"),
    ("createWorkRecordWithTypeAndComment", "Target"),
    ("deleteWorkRecord", "Owned"),
    ("setLocalizedWorkItemTitle", "Target"),
    ("setLocalizedWorkItemDescription", "Target"),
    ("addLinkedItem", "Target"),
    ("addLinkedItemWithRev", "Target"),
    ("removeLinkedItem", "Target"),
    ("addComment", "Owned"),
    ("createCommentNew", "Target"),
    ("setResolvedComment", "Owned"),
    ("setCommentTags", "Owned"),
    ("updateModule", "Target"),
    ("updateBaseline", "Target"),
    ("updateTestRun", "Target"),
    ("updateWikiContentForTestRun", "Target"),
    ("addTestRecordToTestRun", "Target"),
    ("updateTestRecordAtIndex", "Target"),
    ("setTestSteps", "Target"),
    ("addAttachmentToTestRun", "Target"),
    ("updateTestRunAttachment", "Target"),
    ("deleteTestRunAttachment", "Target"),
    ("addAttachmentToTestRecord", "Target"),
    ("deleteAttachmentFromTestRecord", "Target"),
    ("addAttachmentToTestStep", "Target"),
    ("deleteAttachmentFromTestStep", "Target"),
    ("updatePlan", "Target"),
    ("setPlanWikiContent", "Target"),
    ("addPlanItems", "Target"),
    ("removePlanItems", "Target"),
    ("addPlanAllowedType", "Target"),
    ("removePlanAllowedType", "Target"),
    ("createWorkItem", "Create(CreateKind::WorkItem)"),
    ("createDocument", "Create(CreateKind::Document)"),
    ("createTestRun", "Create(CreateKind::TestRun)"),
    ("createTestRunWithTitle", "Create(CreateKind::TestRun)"),
    ("beginTransaction", "Transaction"),
    ("endTransaction", "Transaction"),
];

/// Write operations a write-restricted client never sends, and why.
const NEVER: &[(&str, &str)] = &[
    (
        "performWorkflowAction",
        "workflow actions run workflow functions, which may change other objects",
    ),
    ("resetWorkflow", "a workflow action"),
    (
        "doAutoSuspect",
        "marks links on other work items as suspect",
    ),
    ("doAutoassign", "assigns people by the project's rules"),
    ("deleteWorkItem", "cannot be undone"),
    ("deleteModule", "cannot be undone"),
    ("deleteBaseline", "cannot be undone"),
    ("deletePlans", "cannot be undone"),
    (
        "moveWorkItemToDocument",
        "changes the documents the work item moves between",
    ),
    ("moveWorkItemToModule", "deprecated move"),
    (
        "createWorkItemInModule",
        "changes the document; not in the create budget",
    ),
    (
        "createModule",
        "old-style document; not in the create budget",
    ),
    (
        "reuseDocument",
        "creates a document and work items outside the budget",
    ),
    ("reuseModule", "old-style reuse"),
    (
        "updateDerivedDocument",
        "changes the work items of a derived document",
    ),
    ("updateDerivedModule", "old-style derived update"),
    (
        "updateTitleHeadingInDocument",
        "changes the document's title heading work item",
    ),
    (
        "executeTest",
        "may create defects and change other test results",
    ),
    ("createSummaryDefect", "creates a defect work item"),
    (
        "updateSummaryDefect",
        "creates or changes a defect work item",
    ),
    ("addProjectToProjectSpan", "changes what other projects see"),
    (
        "removeProjectFromProjectSpan",
        "changes what other projects see",
    ),
    (
        "addProjectToPlanProjectSpan",
        "changes what other projects see",
    ),
    (
        "removeProjectFromPlanProjectSpan",
        "changes what other projects see",
    ),
    (
        "addExternallyLinkedItem",
        "links an object on another server",
    ),
    (
        "removeExternallyLinkedItem",
        "unlinks an object on another server",
    ),
    ("addLinkedOslcItem", "links an OSLC resource"),
    ("addLinkedRevision", "links a repository revision"),
    (
        "addExternalLinkedRevision",
        "links a revision of another repository",
    ),
    ("removeLinkedRevision", "unlinks a repository revision"),
    (
        "removeExternalLinkedRevision",
        "unlinks a revision of another repository",
    ),
    ("createPlan", "not in the create budget"),
    ("createPlanTemplate", "not in the create budget"),
    ("createBaseline", "not in the create budget"),
    ("createFolder", "not in the create budget"),
    ("createUser", "global"),
    ("updateUser", "global"),
    ("updateUserAccountVault", "global"),
    (
        "addCommentToActivity",
        "activities are not tied to a target",
    ),
    (
        "createDocumentCommentReferringWI",
        "a document comment that refers to a work item",
    ),
    ("addTestRecord", "not used by the library"),
    ("updateTestRecord", "not used by the library"),
    ("createComment", "deprecated, not used by the library"),
    (
        "createDocumentComment",
        "deprecated, not used by the library",
    ),
    (
        "createDocumentCommentReply",
        "deprecated, not used by the library",
    ),
];

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `(service, operation, read)` of every generated operation, from the `call(...)` arguments.
fn soap_operations() -> Vec<(String, String, bool)> {
    let services = read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../polarion-soap/src/generated/services.rs"
    ));
    let lines: Vec<&str> = services.lines().map(str::trim).collect();
    let string = |line: &str| {
        line.strip_prefix('"')
            .and_then(|l| l.strip_suffix("\","))
            .unwrap_or_else(|| panic!("not a string argument: {line}"))
            .to_owned()
    };
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| **l == ".call(")
        .map(|(i, _)| {
            let read = match lines[i + 3] {
                "true," => true,
                "false," => false,
                other => panic!("not a read flag: {other}"),
            };
            (string(lines[i + 1]), string(lines[i + 2]), read)
        })
        .collect()
}

/// `(operation, class, parameters)` of the library's table of operations allowed in restricted
/// mode (one row per line: the table is not formatted).
fn library_operations() -> Vec<(String, String, Vec<String>)> {
    let source = read(concat!(env!("CARGO_MANIFEST_DIR"), "/src/restricted.rs"));
    let table = source
        .split_once("pub(crate) const OPERATIONS: &[(&str, Class, Subject, &[&str])] = &[")
        .expect("the OPERATIONS table")
        .1;
    let table = &table[..table.find("\n];").expect("the end of OPERATIONS")];
    table
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("(\""))
        .map(|l| {
            let (op, class, params) = l
                .strip_prefix("(\"")
                .and_then(|l| l.split_once("\", Class::"))
                .and_then(|(op, rest)| {
                    let (class, rest) = rest.split_once(", Subject::")?;
                    let params = rest.split_once(", &[")?.1.strip_suffix("]),")?;
                    Some((op, class, params))
                })
                .unwrap_or_else(|| panic!("unexpected OPERATIONS line: {l}"));
            let params = params
                .split(", ")
                .filter(|p| !p.is_empty())
                .map(|p| p.trim_matches('"').to_owned())
                .collect();
            (op.to_owned(), class.to_owned(), params)
        })
        .collect()
}

/// `(operation, class)` of the library's table of operations allowed in restricted mode.
fn library_classes() -> Vec<(String, String)> {
    library_operations()
        .into_iter()
        .map(|(op, class, _)| (op, class))
        .collect()
}

/// The parameter elements of every generated operation, in order (`NAME[]` for an array, which
/// is written as repeated elements), from the body of its service method between
/// `w.open(_, "OP")` and `w.close(_, "OP")`.
fn soap_parameters() -> BTreeMap<String, Vec<String>> {
    let services = read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../polarion-soap/src/generated/services.rs"
    ));
    // The last string argument of `x.write_xml(&mut w, "p", "NAME")` or `w.nil("p", "NAME")`.
    let name = |line: &str| -> Option<String> {
        let args = line
            .split_once(".write_xml(&mut w, ")
            .or_else(|| line.split_once("w.nil("))?
            .1;
        let (_, rest) = args.split_once("\", \"")?;
        Some(rest.split_once('"')?.0.to_owned())
    };
    let mut out = BTreeMap::new();
    let mut lines = services.lines().map(str::trim);
    while let Some(line) = lines.next() {
        let Some(op) = line
            .strip_prefix("w.open(\"")
            .and_then(|l| l.split_once("\", \""))
            .and_then(|(_, op)| op.strip_suffix("\");"))
        else {
            continue;
        };
        let close = format!("\", \"{op}\");");
        let mut params: Vec<String> = Vec::new();
        let mut array = false;
        for line in lines.by_ref() {
            if line.starts_with("w.close(") && line.ends_with(&close) {
                break;
            }
            if line.starts_with("for v in ") {
                array = true;
            }
            if let Some(name) = name(line) {
                let name = if array { format!("{name}[]") } else { name };
                array = false;
                if !params.contains(&name) {
                    params.push(name);
                }
            }
        }
        out.insert(op.to_owned(), params);
    }
    out
}

#[test]
fn the_filter_knows_the_parameters_of_its_operations() {
    let generated = soap_parameters();
    assert!(
        generated.len() > 300,
        "parsed {} operations",
        generated.len()
    );
    assert_eq!(
        generated["addPlanItems"],
        ["uri", "items[]"],
        "the parser of services.rs"
    );
    for (op, _, params) in library_operations() {
        assert_eq!(
            Some(&params),
            generated.get(&op),
            "the parameters of {op} in src/restricted.rs differ from the generated service"
        );
    }
}

#[test]
fn every_write_operation_is_classified() {
    let operations = soap_operations();
    assert!(
        operations.len() > 300,
        "parsed {} operations",
        operations.len()
    );
    let writes: Vec<&str> = operations
        .iter()
        .filter(|(_, _, read)| !read)
        .map(|(_, op, _)| op.as_str())
        .collect();
    // The filter gets the operation name without the service.
    let unique: BTreeSet<&str> = writes.iter().copied().collect();
    assert_eq!(unique.len(), writes.len(), "write operation names repeat");

    let allowed: BTreeMap<&str, &str> = ALLOWED.iter().copied().collect();
    let never: BTreeMap<&str, &str> = NEVER.iter().copied().collect();
    assert_eq!(allowed.len(), ALLOWED.len(), "ALLOWED repeats an operation");
    assert_eq!(never.len(), NEVER.len(), "NEVER repeats an operation");
    let unclassified: Vec<&str> = writes
        .iter()
        .copied()
        .filter(|op| !allowed.contains_key(op) && !never.contains_key(op))
        .collect();
    assert!(
        unclassified.is_empty(),
        "SOAP write operations neither allowed in restricted mode (src/restricted.rs and ALLOWED) \
         nor listed in NEVER in tests/write_targets.rs: {unclassified:?}"
    );
    for op in allowed.keys().chain(never.keys()) {
        assert!(unique.contains(op), "`{op}` is not a SOAP write operation");
    }
    for op in allowed.keys() {
        assert!(!never.contains_key(op), "`{op}` is both allowed and never");
    }

    let library: BTreeMap<String, String> = library_classes().into_iter().collect();
    let expected: BTreeMap<String, String> = ALLOWED
        .iter()
        .map(|(op, class)| ((*op).to_owned(), (*class).to_owned()))
        .collect();
    assert_eq!(
        library, expected,
        "the operations src/restricted.rs allows differ from ALLOWED"
    );
}
