//! The library's request composition and result mapping against a mock SOAP server.

use polarion::{Auth, Error, FieldValue, Fields, Polarion, WorkItemQuery};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
const PROJECT: &str = "/polarion/ws/services/ProjectWebService";
const URI: &str = "subterra:data-service:objects:/default/";

fn envelope(header: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{header}<soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>"#
    )
}

fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(envelope("", body), "text/xml; charset=utf-8")
}

/// Mock server with a token login, and a client for it.
async fn setup() -> (MockServer, Polarion) {
    let server = MockServer::start().await;
    Mock::given(path("/polarion/ws/services/SessionWebService"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            envelope(
                r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">-1</ns1:sessionID></soapenv:Header>"#,
                r#"<logInWithTokenResponse xmlns="http://ws.polarion.com/SessionWebService-impl"/>"#,
            ),
            "text/xml",
        ))
        .mount(&server)
        .await;
    let polarion = Polarion::new(
        &format!("{}/polarion", server.uri()),
        Auth::Token("t".into()),
    )
    .unwrap();
    (server, polarion)
}

async fn mock(server: &MockServer, service: &str, needles: &[&str], body: &str) {
    let mut m = Mock::given(path(service));
    for n in needles {
        m = m.and(body_string_contains(*n));
    }
    m.respond_with(xml(body)).expect(1).mount(server).await;
}

#[tokio::test]
async fn work_item_is_mapped_and_missing_ones_are_not_found() {
    let (server, polarion) = setup().await;
    mock(
        &server,
        TRACKER,
        &["getWorkItemById>", ">P-1<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-1">
<customFields>
  <Custom><key>asil</key><value xsi:type="ns2:EnumOptionId" xmlns:ns2="http://ws.polarion.com/TrackerWebService-types"><id>D</id></value></Custom>
  <Custom><key>empty</key><value xsi:nil="true"/></Custom>
</customFields>
<id>P-1</id>
<linkedWorkItems><LinkedWorkItem><role><id>verifies</id></role><workItemURI>{URI}Q${{WorkItem}}Q-9</workItemURI></LinkedWorkItem></linkedWorkItems>
<moduleURI>{URI}P${{Module}}{{moduleFolder}}Specs/SRS</moduleURI>
<status><id>open</id></status>
<title>Brakes</title>
</getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
    .await;
    mock(
        &server,
        TRACKER,
        &["getWorkItemById>", ">P-404<"],
        &format!(
            r#"<getWorkItemByIdResponse><getWorkItemByIdReturn uri="{URI}P${{WorkItem}}P-404" unresolvable="true"/></getWorkItemByIdResponse>"#
        ),
    )
    .await;

    let item = polarion.work_item("P", "P-1").await.unwrap();
    assert_eq!(item.project.as_deref(), Some("P"));
    assert_eq!(item.status.as_deref(), Some("open"));
    assert_eq!(
        item.custom_fields.get("asil"),
        Some(&FieldValue::Enum("D".into()))
    );
    assert!(!item.custom_fields.contains_key("empty"));
    assert_eq!(item.links[0].item.id.as_deref(), Some("Q-9"));
    assert_eq!(
        item.document.as_ref().and_then(|d| d.location()).as_deref(),
        Some("Specs/SRS")
    );

    let err = polarion.work_item("P", "P-404").await.unwrap_err();
    assert!(
        matches!(&err, Error::NotFound { kind: "work item", id } if id == "P/P-404"),
        "{err:?}"
    );
}

#[tokio::test]
async fn query_sends_project_filter_limit_and_fields() {
    let (server, polarion) = setup().await;
    mock(
        &server,
        TRACKER,
        &[
            "queryWorkItemsLimited>",
            ">project.id:P AND (type:requirement)<",
            "sort>~updated<",
            "resultsLimit>5<",
            "fields>title<",
            "fields>customFields.asil<",
        ],
        r#"<queryWorkItemsLimitedResponse><queryWorkItemsLimitedReturn><id>P-1</id></queryWorkItemsLimitedReturn><queryWorkItemsLimitedReturn><id>P-2</id></queryWorkItemsLimitedReturn></queryWorkItemsLimitedResponse>"#,
    )
    .await;
    mock(
        &server,
        TRACKER,
        &["getWorkItemsCount>", ">project.id:P AND (type:requirement)<"],
        r#"<getWorkItemsCountResponse><getWorkItemsCountReturn>42</getWorkItemsCountReturn></getWorkItemsCountResponse>"#,
    )
    .await;

    let query = WorkItemQuery::new("type:requirement")
        .project("P")
        .sort("~updated")
        .limit(5)
        .fields(Fields::new(["id", "title"]).with_custom("asil"));
    let items = polarion.query_work_items(&query).await.unwrap();
    let ids: Vec<_> = items.iter().filter_map(|i| i.id.as_deref()).collect();
    assert_eq!(ids, ["P-1", "P-2"]);
    assert_eq!(polarion.count_work_items(&query).await.unwrap(), 42);
}

#[tokio::test]
async fn documents_and_their_work_items() {
    let (server, polarion) = setup().await;
    let module = |space: &str, name: &str| {
        format!(
            r#"<moduleFolder>{space}</moduleFolder><moduleName>{name}</moduleName><title>{name} title</title>"#
        )
    };
    mock(
        &server,
        TRACKER,
        &["queryModules>", ">project.id:P<", "resultsLimit>-1<", "fields>moduleFolder<"],
        &format!(
            "<queryModulesResponse><queryModulesReturn>{}</queryModulesReturn><queryModulesReturn>{}</queryModulesReturn></queryModulesResponse>",
            module("Specs", "SRS"),
            module("_default", "Plan"),
        ),
    )
    .await;
    mock(
        &server,
        TRACKER,
        &["getModulesWithFields>", "location>Specs<"],
        &format!(
            "<getModulesWithFieldsResponse><getModulesWithFieldsReturn>{}</getModulesWithFieldsReturn></getModulesWithFieldsResponse>",
            module("Specs", "SRS"),
        ),
    )
    .await;
    mock(
        &server,
        TRACKER,
        &["getModuleByLocation>", "location>Specs/SRS<"],
        &format!(
            r#"<getModuleByLocationResponse><getModuleByLocationReturn uri="{URI}P${{Module}}{{moduleFolder}}Specs/SRS"><homePageContent><type>text/html</type><content>&lt;h1&gt;SRS&lt;/h1&gt;</content><contentLossy>false</contentLossy></homePageContent>{}</getModuleByLocationReturn></getModuleByLocationResponse>"#,
            module("Specs", "SRS"),
        ),
    )
    .await;
    mock(
        &server,
        TRACKER,
        &[
            "getModuleWorkItems>",
            r#"parentWorkItemURI xsi:nil="true"/>"#,
            "deep>true<",
        ],
        r#"<getModuleWorkItemsResponse><getModuleWorkItemsReturn><id>P-2</id><outlineNumber>1.1</outlineNumber></getModuleWorkItemsReturn></getModuleWorkItemsResponse>"#,
    )
    .await;

    let all = polarion.documents("P", None).await.unwrap();
    let names: Vec<_> = all
        .iter()
        .map(|d| {
            format!(
                "{}/{}",
                d.space.as_deref().unwrap(),
                d.name.as_deref().unwrap()
            )
        })
        .collect();
    assert_eq!(names, ["Specs/SRS", "_default/Plan"]);
    assert!(all.iter().all(|d| d.body.is_none()));

    let in_space = polarion.documents("P", Some("Specs")).await.unwrap();
    assert_eq!(in_space.len(), 1);

    let doc = polarion.document("P", "Specs", "SRS").await.unwrap();
    assert_eq!(doc.body.as_ref().unwrap().content, "<h1>SRS</h1>");

    let items = polarion
        .document_work_items(doc.uri.as_deref().unwrap(), &Fields::standard())
        .await
        .unwrap();
    assert_eq!(items[0].outline_number.as_deref(), Some("1.1"));
}

#[tokio::test]
async fn projects_walk_the_group_tree() {
    let (server, polarion) = setup().await;
    let project = |id: &str| {
        format!("<getContainedProjectsReturn><id>{id}</id></getContainedProjectsReturn>")
    };
    mock(
        &server,
        PROJECT,
        &["getRootProjectGroup"],
        r#"<getRootProjectGroupResponse><getRootProjectGroupReturn uri="G0"/></getRootProjectGroupResponse>"#,
    )
    .await;
    mock(
        &server,
        PROJECT,
        &["getContainedProjects>", ">G0<"],
        &format!(
            "<getContainedProjectsResponse>{}</getContainedProjectsResponse>",
            project("B")
        ),
    )
    .await;
    mock(
        &server,
        PROJECT,
        &["getContainedGroups>", ">G0<"],
        r#"<getContainedGroupsResponse><getContainedGroupsReturn uri="G1"/></getContainedGroupsResponse>"#,
    )
    .await;
    mock(
        &server,
        PROJECT,
        &["getContainedProjects>", ">G1<"],
        &format!(
            "<getContainedProjectsResponse>{}</getContainedProjectsResponse>",
            project("A")
        ),
    )
    .await;
    mock(
        &server,
        PROJECT,
        &["getContainedGroups>", ">G1<"],
        "<getContainedGroupsResponse/>",
    )
    .await;

    let ids: Vec<_> = polarion
        .projects()
        .await
        .unwrap()
        .into_iter()
        .filter_map(|p| p.id)
        .collect();
    assert_eq!(ids, ["A", "B"]);
}

#[tokio::test]
async fn new_clients_are_read_only() {
    let (server, polarion) = setup().await;
    let err = polarion
        .soap()
        .tracker()
        .create_work_item(&Default::default())
        .await
        .unwrap_err();
    assert!(matches!(err, polarion::soap::Error::ReadOnly { .. }));
    assert!(server.received_requests().await.unwrap().is_empty());
}
