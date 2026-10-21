//! Documents, wiki, baselines, revisions, activities, test management, planning, security,
//! users and transactions against a mock server, including the write guard.

mod common;

use common::*;
use polarion::{
    ActivityScope, DocumentRef, DocumentUpdate, Error, Fields, ItemRef, NewDocument, PlanUpdate,
    Query, TestRecord, Text,
};
use serde_json::json;

fn refused<T: std::fmt::Debug>(result: polarion::Result<T>, project: Option<&str>) {
    match result {
        Err(Error::WriteNotAllowed { project: p, .. }) => assert_eq!(p.as_deref(), project),
        other => panic!("expected WriteNotAllowed, got {other:?}"),
    }
}

fn text(s: &str) -> Text {
    Text {
        content_type: "text/plain".into(),
        content: s.into(),
    }
}

#[tokio::test]
async fn guards_cover_every_area() {
    let server = server().await;
    let writer = writer(&server); // Sandbox only, no global writes
    let prod_run = format!("{URI}Prod${{TestRun}}R1");

    refused(writer.users().create("jdoe").await, None);
    refused(writer.folders().create(None, None, "f", "F").await, None);
    refused(
        writer.folders().create(Some("Prod"), None, "f", "F").await,
        Some("Prod"),
    );
    refused(
        writer.activities().add_comment("a-1", &text("x")).await,
        None,
    );
    refused(
        writer
            .security()
            .update_user_account_vault("k", "u", "p")
            .await,
        None,
    );
    refused(
        writer.baselines().create("Prod", "B", None, None).await,
        Some("Prod"),
    );
    refused(writer.baselines().delete("junk").await, None);
    refused(
        writer.tests().set_report(&prod_run, &text("x")).await,
        Some("Prod"),
    );
    refused(
        writer
            .plans()
            .add_items(&format!("{URI}Prod${{Plan}}P1"), &[])
            .await,
        Some("Prod"),
    );
    refused(
        writer
            .documents()
            .update(
                &DocumentRef::new("Prod", "Specs", "SRS"),
                &DocumentUpdate::default(),
            )
            .await,
        Some("Prod"),
    );
    refused(
        writer
            .documents()
            .create(&NewDocument {
                project: "Prod".into(),
                name: "X".into(),
                structure_link_role: "parent".into(),
                ..NewDocument::default()
            })
            .await,
        Some("Prod"),
    );
    refused(reader(&server).begin_transaction().await, None);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn documents_create_update_query_comment_export() {
    let server = server().await;
    let doc = DocumentRef::new("Sandbox", "Specs", "SRS");
    expect(
        &server,
        TRACKER,
        &[
            "createDocument>",
            "projectId>Sandbox<",
            r#"location xsi:nil="true"/>"#,
            "documentName>SRS<",
            "structureLinkRole><trackert:id>parent<",
            "allowedWITypes><trackert:id>requirement<",
        ],
        &format!("<createDocumentResponse><createDocumentReturn>{}</createDocumentReturn></createDocumentResponse>", doc.uri),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "updateModule>",
            &format!(r#"module uri="{}""#, doc.uri),
            "title>New<",
        ],
        "<updateModuleResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["queryModulesInBaseline>", "baselineRevision>77<", "fields>moduleName<"],
        "<queryModulesInBaselineResponse><queryModulesInBaselineReturn><moduleFolder>Specs</moduleFolder><moduleName>SRS</moduleName></queryModulesInBaselineReturn></queryModulesInBaselineResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["createDocumentCommentReferringWI>", "workItemURI>", "text>"],
        "<createDocumentCommentReferringWIResponse><createDocumentCommentReferringWIReturn>c-1</createDocumentCommentReferringWIReturn></createDocumentCommentReferringWIResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["exportDocumentToPDF>", "paperSize>A4<"],
        "<exportDocumentToPDFResponse><exportDocumentToPDFReturn>JVBERi0=</exportDocumentToPDFReturn></exportDocumentToPDFResponse>",
    )
    .await;

    let polarion = writer(&server);
    let docs = polarion.documents();
    let created = docs
        .create(&NewDocument {
            project: "Sandbox".into(),
            name: "SRS".into(),
            allowed_types: vec!["requirement".into()],
            structure_link_role: "parent".into(),
            ..NewDocument::default()
        })
        .await
        .unwrap();
    assert_eq!(created, doc);
    docs.update(
        &doc,
        &DocumentUpdate {
            title: Some("New".into()),
            ..DocumentUpdate::default()
        },
    )
    .await
    .unwrap();
    let found = docs
        .query(&Query::new("type:spec").baseline("77"))
        .await
        .unwrap();
    assert_eq!(found[0].name.as_deref(), Some("SRS"));
    let comment = docs
        .add_comment(&doc, Some(&ItemRef::new("Sandbox", "SB-1")), &text("check"))
        .await
        .unwrap();
    assert_eq!(comment, "c-1");
    let pdf = docs
        .export_pdf(
            &doc,
            &polarion::soap::types::PdfProperties {
                paper_size: "A4".into(),
                orientation: "portrait".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(pdf, b"%PDF-");
}

#[tokio::test]
async fn test_runs_records_and_steps() {
    let server = server().await;
    let run = format!("{URI}Sandbox${{TestRun}}R1");
    expect(
        &server,
        TESTS,
        &["getTestRunByUri>", &format!("uri>{run}<")],
        &format!(
            r#"<getTestRunByUriResponse><getTestRunByUriReturn uri="{run}"><id>R1</id><isTemplate>false</isTemplate><projectURI>{URI}Sandbox${{Project}}Sandbox</projectURI>
<records><TestRecord><executedByURI>{URI}${{User}}amy</executedByURI><result><id>passed</id></result><testCaseURI>{URI}Sandbox${{WorkItem}}SB-5</testCaseURI></TestRecord></records>
<status><id>finished</id></status><title>Nightly</title></getTestRunByUriReturn></getTestRunByUriResponse>"#
        ),
    )
    .await;
    expect(
        &server,
        TESTS,
        &[
            "searchTestRunsWithFieldsLimited>",
            "fields>title<",
            "resultsLimit>5<",
        ],
        "<searchTestRunsWithFieldsLimitedResponse/>",
    )
    .await;
    expect(
        &server,
        TESTS,
        &[
            "addTestRecordToTestRun>",
            &format!("testCaseURI>{URI}Sandbox${{WorkItem}}SB-5<"),
            "result><trackert:id>failed<",
            &format!("executedByURI>{URI}${{User}}amy<"),
        ],
        "<addTestRecordToTestRunResponse/>",
    )
    .await;
    expect(
        &server,
        TESTS,
        &["setTestSteps>", "content>Open the door<"],
        "<setTestStepsResponse/>",
    )
    .await;

    let polarion = writer(&server);
    let tests = polarion.tests();
    let got = tests.get_by_uri(&run, None).await.unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        json!({
            "id": "R1", "project": "Sandbox", "title": "Nightly", "status": "finished",
            "records": [{"testCase": {"project": "Sandbox", "id": "SB-5"}, "result": "passed", "executedBy": "amy"}],
            "uri": run,
        })
    );
    tests
        .search(
            &Query::new("status:open")
                .limit(5)
                .fields(Fields::new(["title"])),
        )
        .await
        .unwrap();
    tests
        .add_record(
            &run,
            &TestRecord {
                test_case: Some(ItemRef::new("Sandbox", "SB-5")),
                result: Some("failed".into()),
                executed_by: Some("amy".into()),
                ..TestRecord::default()
            },
        )
        .await
        .unwrap();
    tests
        .set_steps(
            &ItemRef::new("Sandbox", "SB-5"),
            &[vec![text("Open the door"), text("It opens")]],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn plans_wiki_baselines_revisions_activities_security() {
    let server = server().await;
    let plan = format!("{URI}Sandbox${{Plan}}rel1");
    expect(
        &server,
        PLANNING,
        &["searchPlans>", "resultsLimit>-1<"],
        &format!(
            r#"<searchPlansResponse><searchPlansReturn uri="{plan}"><id>rel1</id><name>Release 1</name><records><PlanRecord><item uri="{URI}Sandbox${{WorkItem}}SB-1"/></PlanRecord></records></searchPlansReturn></searchPlansResponse>"#
        ),
    )
    .await;
    expect(
        &server,
        PLANNING,
        &["updatePlan>", "name>Release 1.1<"],
        "<updatePlanResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["queryWikiPages>", "query>project.id:Sandbox<"],
        "<queryWikiPagesResponse><queryWikiPagesReturn><pageName>Home</pageName><spaceId>_default</spaceId></queryWikiPagesReturn></queryWikiPagesResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["createBaseline>", "name>B1<", r#"revision xsi:nil="true"/>"#],
        "<createBaselineResponse><createBaselineReturn><baseRevision>1234</baseRevision><id>b1</id><name>B1</name></createBaselineReturn></createBaselineResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["getRevision>", "repositoryName>default<", "revisionId>1234<"],
        &format!("<getRevisionResponse><getRevisionReturn><author>amy</author><linkedWorkItemURIs><SubterraURI>{URI}Sandbox${{WorkItem}}SB-1</SubterraURI></linkedWorkItemURIs><message>fix</message><name>1234</name></getRevisionReturn></getRevisionResponse>"),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "listProjectActivities>",
            "projectId>Sandbox<",
            r#"query xsi:nil="true"/>"#,
            "count>10<",
        ],
        "<listProjectActivitiesResponse/>",
    )
    .await;
    expect(
        &server,
        SECURITY,
        &["canModifyInstance>"],
        "<canModifyInstanceResponse><canModifyInstanceReturn>true</canModifyInstanceReturn></canModifyInstanceResponse>",
    )
    .await;

    let polarion = writer(&server);
    let plans = polarion
        .plans()
        .search(&Query::new("").unlimited())
        .await
        .unwrap();
    assert_eq!(plans[0].items, [ItemRef::new("Sandbox", "SB-1")]);
    polarion
        .plans()
        .update(
            &plan,
            &PlanUpdate {
                name: Some("Release 1.1".into()),
                ..PlanUpdate::default()
            },
        )
        .await
        .unwrap();
    let pages = polarion
        .wiki()
        .query(&Query::new("").project("Sandbox"))
        .await
        .unwrap();
    assert_eq!(pages[0].name.as_deref(), Some("Home"));
    let baseline = polarion
        .baselines()
        .create("Sandbox", "B1", None, None)
        .await
        .unwrap();
    assert_eq!(baseline.revision.as_deref(), Some("1234"));
    let revision = polarion.revisions().get("default", "1234").await.unwrap();
    assert_eq!(
        revision.linked_work_items,
        [ItemRef::new("Sandbox", "SB-1")]
    );
    polarion
        .activities()
        .list(ActivityScope::Project("Sandbox"), None, 10)
        .await
        .unwrap();
    assert!(
        polarion
            .security()
            .can_modify(&ItemRef::new("Sandbox", "SB-1").uri)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn global_writes_and_transactions() {
    let server = server().await;
    expect(
        &server,
        PROJECT,
        &["createUser>", "in0>jdoe<"],
        "<createUserResponse><createUserReturn><id>jdoe</id></createUserReturn></createUserResponse>",
    )
    .await;
    expect(
        &server,
        SESSION,
        &["beginTransaction>"],
        "<beginTransactionResponse/>",
    )
    .await;
    expect(
        &server,
        SESSION,
        &["endTransaction>", "rollback>true<"],
        "<endTransactionResponse/>",
    )
    .await;

    let polarion = admin(&server);
    let user = polarion.users().create("jdoe").await.unwrap();
    assert_eq!(user.id.as_deref(), Some("jdoe"));
    polarion.begin_transaction().await.unwrap();
    polarion.rollback().await.unwrap();
}

#[tokio::test]
async fn writes_name_their_objects_by_uri() {
    let server = server().await;
    let baseline = format!("{URI}Sandbox${{Baseline}}b1");
    expect(
        &server,
        TRACKER,
        &[
            "updateBaseline>",
            &format!(r#"baseline uri="{baseline}""#),
            "name>B1<",
        ],
        "<updateBaselineResponse/>",
    )
    .await;
    let run = format!("{URI}Sandbox${{TestRun}}R1");
    let case = ItemRef::new("Sandbox", "SB-5");
    expect(
        &server,
        TESTS,
        &[
            "updateTestRecordAtIndex>",
            &format!("testRunUri>{run}<"),
            "index>0<",
            &format!("testCaseURI>{}<", case.uri),
            "result><trackert:id>passed<",
            &format!("executedByURI>{}<", polarion::uri::user_uri("amy")),
        ],
        "<updateTestRecordAtIndexResponse/>",
    )
    .await;
    let document = polarion::DocumentRef::new("Sandbox", "Specs", "SRS");
    let parent = ItemRef::new("Sandbox", "SB-1");
    expect(
        &server,
        PROJECT,
        &["getProject>", ">Sandbox<"],
        &format!(
            r#"<getProjectResponse><getProjectReturn uri="{URI}Sandbox${{Project}}Sandbox"><id>Sandbox</id></getProjectReturn></getProjectResponse>"#
        ),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "createWorkItemInModule>",
            &format!("moduleURI>{}<", document.uri),
            &format!("parentWorkItemURI>{}<", parent.uri),
            &format!(r#"project uri="{URI}Sandbox${{Project}}Sandbox""#),
            "type><trackert:id>requirement<",
            "title>Brakes<",
        ],
        &format!(
            "<createWorkItemInModuleResponse><createWorkItemInModuleReturn>{URI}Sandbox${{WorkItem}}SB-7</createWorkItemInModuleReturn></createWorkItemInModuleResponse>"
        ),
    )
    .await;

    let polarion = writer(&server);
    polarion
        .baselines()
        .update(&baseline, Some("B1"), None)
        .await
        .unwrap();
    let record = polarion::TestRecord {
        test_case: Some(case),
        result: Some("passed".into()),
        executed_by: Some("amy".into()),
        ..polarion::TestRecord::default()
    };
    polarion
        .tests()
        .update_record(&run, 0, &record)
        .await
        .unwrap();
    let new = polarion::NewWorkItem::new("Sandbox", "requirement", "Brakes");
    let created = polarion
        .items()
        .create_in_document(&new, &document, Some(&parent))
        .await
        .unwrap();
    assert_eq!(created, ItemRef::new("Sandbox", "SB-7"));
    // The baseline is named by its URI only.
    let sent = &bodies(&server, "updateBaseline").await[0];
    assert!(!sent.contains(">b1<"), "{sent}");
}

#[tokio::test]
async fn users_are_updated_by_uri_only() {
    let server = server().await;
    let user_uri = polarion::uri::user_uri("jdoe");
    expect(
        &server,
        PROJECT,
        &["getUser>", "userID>jdoe<"],
        &format!(
            r#"<getUserResponse><getUserReturn uri="{user_uri}"><id>jdoe</id><name>J</name></getUserReturn></getUserResponse>"#
        ),
    )
    .await;
    expect(
        &server,
        PROJECT,
        &[
            "updateUser>",
            &format!(r#"in0 uri="{user_uri}""#),
            "name>Jane<",
        ],
        "<updateUserResponse/>",
    )
    .await;
    let update = polarion::UserUpdate {
        name: Some("Jane".into()),
        ..polarion::UserUpdate::default()
    };
    admin(&server)
        .users()
        .update("jdoe", &update)
        .await
        .unwrap();

    // Without its URI, the user would be a new one to create: nothing is sent.
    let server = common::server().await;
    expect(
        &server,
        PROJECT,
        &["getUser>"],
        "<getUserResponse><getUserReturn><id>jdoe</id></getUserReturn></getUserResponse>",
    )
    .await;
    let err = admin(&server)
        .users()
        .update("jdoe", &update)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m.contains("jdoe")),
        "{err:?}"
    );
    assert!(bodies(&server, "updateUser").await.is_empty());
}

#[test]
fn soap_types_serialize_to_json() {
    let folder = polarion::soap::types::Folder {
        name: Some("Specs".into()),
        title: Some("Specifications".into()),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(&folder).unwrap(),
        json!({"name": "Specs", "title": "Specifications"})
    );
}
