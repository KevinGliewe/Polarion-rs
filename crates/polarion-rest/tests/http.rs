//! Request/response plumbing of generated operations, against a mock server.

use polarion_rest::models::{WorkItem, WorkItemAttachmentAttributes, WorkItemAttributes};
use polarion_rest::{
    Client, Error, FilePart, Query, RelationshipData, Resource, ResourceKind, ResourceRef,
    TextContent,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

async fn setup() -> (MockServer, Client) {
    let server = MockServer::start().await;
    let client = Client::new(&format!("{}/polarion", server.uri()), "secret-token").unwrap();
    (server, client)
}

#[tokio::test]
async fn list_sends_auth_and_query_and_keeps_custom_fields() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path("/polarion/rest/v1/projects/PROJ/workitems"))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("accept", "application/json"))
        .and(query_param("fields[workitems]", "@all"))
        .and(query_param("query", "type:task"))
        .and(query_param("page[size]", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": {"totalCount": 1},
            "data": [{
                "type": "workitems",
                "id": "PROJ/WI-1",
                "attributes": {
                    "id": "WI-1",
                    "title": "Brake",
                    "description": {"type": "text/html", "value": "<p>x</p>"},
                    "asil": "D"
                },
                "relationships": {
                    "author": {"data": {"type": "users", "id": "jdoe"}}
                }
            }],
            "links": {"self": "..."}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = client
        .get_work_items(
            "PROJ",
            &Query::new()
                .all_fields(WorkItemAttributes::TYPE)
                .query("type:task")
                .page_size(10),
        )
        .await
        .unwrap();

    assert_eq!(page.total_count(), Some(1));
    assert!(!page.has_next_page());
    let item = &page.data[0];
    let attrs = item.attributes.as_ref().unwrap();
    assert_eq!(attrs.title.as_deref(), Some("Brake"));
    assert_eq!(attrs.description, Some(TextContent::html("<p>x</p>")));
    assert_eq!(attrs.extra.get("asil"), Some(&json!("D")));
    assert_eq!(
        item.relationship("author").unwrap().refs(),
        &[ResourceRef::new("users", "jdoe")]
    );
}

#[tokio::test]
async fn create_wraps_resources_in_data() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path("/polarion/rest/v1/projects/PROJ/workitems"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "data": [{
                "type": "workitems",
                "attributes": {"type": "task", "title": "New", "asil": "B"},
                "relationships": {"assignee": {"data": [{"type": "users", "id": "jdoe"}]}}
            }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "data": [{"type": "workitems", "id": "PROJ/WI-2", "links": {"self": "..."}}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut attrs = WorkItemAttributes {
        type_: Some("task".into()),
        title: Some("New".into()),
        ..Default::default()
    };
    attrs.extra.insert("asil".into(), json!("B"));
    let item: WorkItem = Resource::from_attributes(attrs)
        .with_relationships("assignee", [ResourceRef::new("users", "jdoe")]);

    let created = client.post_work_items("PROJ", &[item]).await.unwrap();
    assert_eq!(created.data[0].id.as_deref(), Some("PROJ/WI-2"));
}

#[tokio::test]
async fn patch_returns_unit_on_204_and_passes_workflow_action() {
    let (server, client) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/polarion/rest/v1/projects/PROJ/workitems/WI-1"))
        .and(query_param("workflowAction", "close"))
        .and(body_json(json!({
            "data": {"type": "workitems", "id": "PROJ/WI-1", "attributes": {"status": "closed"}}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let update = WorkItem::from_attributes(WorkItemAttributes {
        status: Some("closed".into()),
        ..Default::default()
    })
    .with_id("PROJ/WI-1");
    client
        .patch_work_item(
            "PROJ",
            "WI-1",
            &Query::new().workflow_action("close"),
            &update,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn bulk_delete_sends_refs() {
    let (server, client) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/polarion/rest/v1/projects/PROJ/workitems"))
        .and(body_json(
            json!({"data": [{"type": "workitems", "id": "PROJ/WI-1"}]}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client
        .delete_work_items("PROJ", &[WorkItem::reference("PROJ/WI-1")])
        .await
        .unwrap();
}

#[tokio::test]
async fn api_errors_are_decoded() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errors": [{"status": "404", "title": "Not Found", "detail": "Work Item WI-9 not found"}]
        })))
        .mount(&server)
        .await;

    let err = client
        .get_work_item("PROJ", "WI-9", &Query::new())
        .await
        .unwrap_err();
    assert!(err.is_not_found());
    assert!(matches!(&err, Error::Api { errors, .. } if errors.len() == 1));
    assert_eq!(
        err.to_string(),
        "Polarion API error 404 Not Found: Not Found: Work Item WI-9 not found"
    );
}

#[tokio::test]
async fn non_json_errors_keep_the_body() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("<html>login</html>"))
        .mount(&server)
        .await;

    let err = client.get_projects(&Query::new()).await.unwrap_err();
    assert!(
        matches!(&err, Error::Api { errors, body, .. } if errors.is_empty() && body.contains("login"))
    );
}

#[tokio::test]
async fn multipart_sends_resource_before_files() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path(
            "/polarion/rest/v1/projects/PROJ/workitems/WI-1/attachments",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "data": [{"type": "workitem_attachments", "id": "PROJ/WI-1/1-a.txt"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let resource = Resource::from_attributes(WorkItemAttachmentAttributes {
        file_name: Some("a.txt".into()),
        ..Default::default()
    })
    .with_lid("f1");
    let created = client
        .post_work_item_attachments(
            "PROJ",
            "WI-1",
            Some(&[resource]),
            vec![FilePart::new("a.txt", b"hello".to_vec()).with_mime_type("text/plain")],
        )
        .await
        .unwrap();
    assert_eq!(created.data[0].id.as_deref(), Some("PROJ/WI-1/1-a.txt"));

    let requests: Vec<Request> = server.received_requests().await.unwrap();
    let req = &requests[0];
    let content_type = req.headers.get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    let body = String::from_utf8_lossy(&req.body);
    let resource_at = body.find("name=\"resource\"").expect("resource part");
    let file_at = body
        .find("name=\"files\"; filename=\"a.txt\"")
        .expect("file part");
    assert!(
        resource_at < file_at,
        "resource part must come first:\n{body}"
    );
    assert!(body.contains(r#""lid":"f1""#));
    assert!(body.contains("hello"));
}

#[tokio::test]
async fn binary_content_is_returned_as_bytes() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path(
            "/polarion/rest/v1/projects/PROJ/workitems/WI-1/attachments/1-a.bin/content",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8, 159, 146, 150]))
        .mount(&server)
        .await;

    let bytes = client
        .get_work_item_attachment_content("PROJ", "WI-1", "1-a.bin", &Query::new())
        .await
        .unwrap();
    assert_eq!(bytes, vec![0u8, 159, 146, 150]);
}

#[tokio::test]
async fn path_segments_are_percent_encoded() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path(
            "/polarion/rest/v1/projects/PROJ/spaces/_default/documents/System%20Spec",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"type": "documents", "id": "PROJ/_default/System Spec",
                     "attributes": {"moduleName": "System Spec", "title": "System Spec"}}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let doc = client
        .get_document("PROJ", "_default", "System Spec", &Query::new())
        .await
        .unwrap();
    let attrs = doc.data.attributes.unwrap();
    assert_eq!(attrs.module_name.as_deref(), Some("System Spec"));
}

#[tokio::test]
async fn paginate_follows_next_links() {
    let (server, client) = setup().await;
    let page = |n: u32, next: bool| {
        let ids: Vec<Value> = (0..2)
            .map(|i| json!({"type": "workitems", "id": format!("P/WI-{}", n * 10 + i)}))
            .collect();
        let mut links = json!({"self": "..."});
        if next {
            links["next"] = json!("...");
        }
        json!({"data": ids, "links": links, "meta": {"totalCount": 5}})
    };
    for (n, next) in [(1, true), (2, true), (3, false)] {
        Mock::given(method("GET"))
            .and(path("/polarion/rest/v1/projects/P/workitems"))
            .and(query_param("page[number]", n.to_string()))
            .and(query_param("page[size]", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(n, next)))
            .expect(1)
            .mount(&server)
            .await;
    }

    let all = client
        .paginate(Query::new().page_size(2), async |q| {
            client.get_work_items("P", &q).await
        })
        .await
        .unwrap();
    let ids: Vec<_> = all.data.iter().filter_map(|r| r.id.as_deref()).collect();
    assert_eq!(
        ids,
        [
            "P/WI-10", "P/WI-11", "P/WI-20", "P/WI-21", "P/WI-30", "P/WI-31"
        ]
    );
}

#[tokio::test]
async fn relationship_post_with_only_default_response_returns_unit() {
    // Polarion 2410's spec declares no success response for this operation;
    // the generator falls back to `default` and discards the body.
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path(
            "/polarion/rest/v1/projects/PROJ/collections/C1/relationships/documents",
        ))
        .and(body_json(json!({
            "data": [{"type": "documents", "id": "PROJ/_default/Spec"}]
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client
        .post_collections_relationships(
            "PROJ",
            "C1",
            "documents",
            &RelationshipData::ToMany(vec![ResourceRef::new("documents", "PROJ/_default/Spec")]),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn read_only_client_never_sends_writes() {
    let server = MockServer::start().await;
    let client = Client::builder(&format!("{}/polarion", server.uri()), "t")
        .read_only(true)
        .build()
        .unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(1)
        .mount(&server)
        .await;

    client.get_projects(&Query::new()).await.unwrap();

    let err = client
        .delete_work_items("PROJ", &[WorkItem::reference("PROJ/WI-1")])
        .await
        .unwrap_err();
    assert!(matches!(err, Error::ReadOnly { ref method, .. } if method == "DELETE"));
    let err = client
        .request_json(
            polarion_rest::Method::POST,
            "projects/PROJ/workitems",
            &Query::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::ReadOnly { .. }));
    // Only the GET reached the server.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
