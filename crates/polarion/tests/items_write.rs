//! Work item writes: the allowlist guard and the requests each write sends.

mod common;

use std::collections::BTreeMap;

use common::*;
use polarion::{
    Auth, DocumentRef, Error, FieldValue, ItemRef, NewWorkItem, NewWorkRecord, Polarion, Text,
    WorkItemUpdate,
};

fn sandbox(id: &str) -> ItemRef {
    ItemRef::new("Sandbox", id)
}

fn refused(result: polarion::Result<impl std::fmt::Debug>, project: Option<&str>) {
    match result {
        Err(Error::WriteNotAllowed { project: p, .. }) => assert_eq!(p.as_deref(), project),
        other => panic!("expected WriteNotAllowed, got {other:?}"),
    }
}

#[tokio::test]
async fn writes_are_refused_before_anything_is_sent() {
    let server = server().await;

    // Read-only client: everything refused.
    let reader = reader(&server);
    let items = reader.items();
    refused(items.delete(&sandbox("SB-1")).await, Some("Sandbox"));
    refused(
        items
            .create(&NewWorkItem::new("Sandbox", "task", "x"))
            .await,
        Some("Sandbox"),
    );
    refused(
        items.transition(&sandbox("SB-1"), "done").await,
        Some("Sandbox"),
    );

    // Writer for Sandbox: other projects and unparsable refs are refused.
    let writer = writer(&server);
    let items = writer.items();
    refused(
        items
            .add_assignee(&ItemRef::new("Prod", "PR-1"), "jdoe")
            .await,
        Some("Prod"),
    );
    refused(
        items.create(&NewWorkItem::new("Prod", "task", "x")).await,
        Some("Prod"),
    );
    refused(
        items
            .move_to_document(
                &sandbox("SB-1"),
                &DocumentRef::new("Prod", "Specs", "SRS"),
                None,
                None,
                false,
            )
            .await,
        Some("Prod"),
    );
    refused(
        items
            .update(
                &ItemRef::from_uri("not a polarion uri"),
                &WorkItemUpdate::default(),
            )
            .await,
        None,
    );

    // A wrapped SOAP client that may write: the library still refuses.
    let soap = polarion::soap::Client::new(&base_url(&server), Auth::Token("t".into())).unwrap();
    refused(
        Polarion::from_client(soap)
            .items()
            .delete(&sandbox("SB-1"))
            .await,
        Some("Sandbox"),
    );

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test]
async fn create_sends_project_type_and_typed_custom_fields() {
    let server = server().await;
    expect(
        &server,
        PROJECT,
        &["getProject>", ">Sandbox<"],
        &format!(
            r#"<getProjectResponse><getProjectReturn uri="{URI}Sandbox${{Project}}Sandbox"><id>Sandbox</id><name>Sandbox</name></getProjectReturn></getProjectResponse>"#
        ),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "createWorkItem>",
            &format!(r#"project uri="{URI}Sandbox${{Project}}Sandbox""#),
            "type><trackert:id>requirement<",
            "title>Brakes &amp; steering<",
            "key>asil<",
            r#"xsi:type="trackert:EnumOptionId"><trackert:id>D<"#,
            r#"xsi:type="xsd:date">2024-01-31<"#,
        ],
        &format!(
            "<createWorkItemResponse><createWorkItemReturn>{URI}Sandbox${{WorkItem}}SB-7</createWorkItemReturn></createWorkItemResponse>"
        ),
    )
    .await;

    let mut new = NewWorkItem::new("Sandbox", "requirement", "Brakes & steering");
    new.custom_fields
        .insert("asil".into(), FieldValue::Enum("D".into()));
    new.custom_fields
        .insert("reviewed".into(), FieldValue::Date("2024-01-31".into()));
    let created = writer(&server).items().create(&new).await.unwrap();
    assert_eq!(created, sandbox("SB-7"));
}

#[tokio::test]
async fn update_splits_standard_custom_and_cleared_fields() {
    let server = server().await;
    for (op, response) in [
        ("updateWorkItem>", "<updateWorkItemResponse/>"),
        ("setCustomField>", "<setCustomFieldResponse/>"),
        ("setFieldsNull>", "<setFieldsNullResponse/>"),
    ] {
        wiremock::Mock::given(wiremock::matchers::path(TRACKER))
            .and(wiremock::matchers::body_string_contains(op))
            .respond_with(xml(response))
            .mount(&server)
            .await;
    }

    let update = WorkItemUpdate {
        title: Some("New title".into()),
        description: Some(Text {
            content_type: "text/html".into(),
            content: "<p>x</p>".into(),
        }),
        custom_fields: BTreeMap::from([
            (
                "platforms".to_owned(),
                Some(FieldValue::List(vec![
                    FieldValue::Enum("a".into()),
                    FieldValue::Enum("b".into()),
                ])),
            ),
            ("risk".to_owned(), None),
        ]),
        clear: vec!["dueDate".into()],
        ..WorkItemUpdate::default()
    };
    writer(&server)
        .items()
        .update(&sandbox("SB-1"), &update)
        .await
        .unwrap();

    let updates = bodies(&server, "updateWorkItem").await;
    assert_eq!(updates.len(), 1);
    let body = &updates[0];
    assert!(
        body.contains(&format!(r#"content uri="{URI}Sandbox${{WorkItem}}SB-1""#)),
        "{body}"
    );
    assert!(body.contains("title>New title<"));
    assert!(body.contains("type>text/html<"));
    assert!(
        !body.contains("customFields"),
        "custom fields go through setCustomField: {body}"
    );
    assert!(!body.contains("status"), "{body}");

    let custom = bodies(&server, "setCustomField").await;
    assert_eq!(custom.len(), 1);
    assert!(custom[0].contains("key>platforms<"));
    assert!(custom[0].contains(&format!("parentItemURI>{URI}Sandbox${{WorkItem}}SB-1<")));
    assert!(
        custom[0].contains(r#"xsi:type="trackert:ArrayOfEnumOptionId""#),
        "{}",
        custom[0]
    );

    let cleared = bodies(&server, "setFieldsNull").await;
    assert_eq!(cleared.len(), 1);
    assert!(cleared[0].contains("fields>risk<") && cleared[0].contains("fields>dueDate<"));
}

#[tokio::test]
async fn invalid_values_fail_before_sending() {
    let server = server().await;
    let update = WorkItemUpdate {
        title: Some("x".into()),
        custom_fields: BTreeMap::from([(
            "weird".to_owned(),
            Some(FieldValue::Other {
                xsi_type: Some("Table".into()),
                xml: None,
            }),
        )]),
        ..WorkItemUpdate::default()
    };
    let err = writer(&server)
        .items()
        .update(&sandbox("SB-1"), &update)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert_eq!(calls(&server).await, 0);
}

#[tokio::test]
async fn transition_performs_the_action_leading_to_the_status() {
    let server = server().await;
    wiremock::Mock::given(wiremock::matchers::path(TRACKER))
        .and(wiremock::matchers::body_string_contains("getAvailableActions>"))
        .respond_with(xml(
            r#"<getAvailableActionsResponse>
<getAvailableActionsReturn><actionId>3</actionId><actionName>Review</actionName><nativeActionId>review</nativeActionId><targetStatus><id>reviewed</id></targetStatus></getAvailableActionsReturn>
<getAvailableActionsReturn><actionId>5</actionId><actionName>Close</actionName><nativeActionId>close</nativeActionId><targetStatus><id>done</id></targetStatus><requiredFeatures><item>resolution</item></requiredFeatures></getAvailableActionsReturn>
</getAvailableActionsResponse>"#,
        ))
        .mount(&server)
        .await;
    expect(
        &server,
        TRACKER,
        &["performWorkflowAction>", "actionId>3<"],
        "<performWorkflowActionResponse/>",
    )
    .await;

    let polarion = writer(&server);
    let items = polarion.items();
    let action = items
        .transition(&sandbox("SB-1"), "reviewed")
        .await
        .unwrap();
    assert_eq!((action.id, action.name.as_deref()), (3, Some("Review")));

    let err = items
        .transition(&sandbox("SB-1"), "rejected")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m.contains("Sandbox/SB-1") && m.contains("rejected")),
        "{err:?}"
    );
}

#[tokio::test]
async fn links_comments_people_and_more() {
    let server = server().await;
    let item = sandbox("SB-1");
    let item_uri = item.uri.clone();
    let bool_true =
        |op: &str| format!("<{op}Response><{op}Return>true</{op}Return></{op}Response>");

    expect(
        &server,
        TRACKER,
        &[
            "addLinkedItem>",
            &format!("workitemURI>{item_uri}<"),
            &format!("linkedWorkitemURI>{URI}Other${{WorkItem}}OT-2<"),
            "role><trackert:id>relates_to<",
        ],
        &bool_true("addLinkedItem"),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["addComment>", "title>Note<", "content>Looks good<"],
        &format!("<addCommentResponse><addCommentReturn>{item_uri}#c1</addCommentReturn></addCommentResponse>"),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["addAssignee>", "assigneeId>jdoe<"],
        &bool_true("addAssignee"),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "editApproval>",
            "approveeId>jdoe<",
            "status><trackert:id>approved<",
        ],
        "<editApprovalResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "moveWorkItemToDocument>",
            &format!("documentURI>{URI}Sandbox${{Module}}{{moduleFolder}}Specs/SRS<"),
            r#"parentWorkItemURI xsi:nil="true"/>"#,
            "position>-1<",
            "retainDocumentFlow>false<",
        ],
        "<moveWorkItemToDocumentResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["createWorkRecord>", "date>2024-01-31<", "timeSpent>2h<"],
        "<createWorkRecordResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &[
            "updateAttachment>",
            "id>att-1<",
            r#"fileName xsi:nil="true"/>"#,
            "title>New<",
        ],
        "<updateAttachmentResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["deleteWorkItem>", &format!("workitemURI>{item_uri}<")],
        "<deleteWorkItemResponse/>",
    )
    .await;

    let polarion = writer(&server);
    let items = polarion.items();
    assert!(
        items
            .add_link(&item, "relates_to", &ItemRef::new("Other", "OT-2"))
            .await
            .unwrap()
    );
    let text = Text {
        content_type: "text/plain".into(),
        content: "Looks good".into(),
    };
    assert_eq!(
        items.add_comment(&item, Some("Note"), &text).await.unwrap(),
        format!("{item_uri}#c1")
    );
    assert!(items.add_assignee(&item, "jdoe").await.unwrap());
    items.set_approval(&item, "jdoe", "approved").await.unwrap();
    items
        .move_to_document(
            &item,
            &DocumentRef::new("Sandbox", "Specs", "SRS"),
            None,
            None,
            false,
        )
        .await
        .unwrap();
    items
        .add_work_record(
            &item,
            &NewWorkRecord {
                user: "jdoe".into(),
                date: "2024-01-31".into(),
                time_spent: "2h".into(),
                ..NewWorkRecord::default()
            },
        )
        .await
        .unwrap();
    items
        .update_attachment(&item, "att-1", None, Some("New"), None)
        .await
        .unwrap();
    items.delete(&item).await.unwrap();
}
