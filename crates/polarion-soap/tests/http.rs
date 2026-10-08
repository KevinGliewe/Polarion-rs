//! Session handling, faults and decoding against a mock SOAP server.

use polarion_soap::types::{AnyValue, WorkItem};
use polarion_soap::{Auth, Client, Error};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENVELOPE_START: &str = r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#;

fn envelope(header: &str, body: &str) -> String {
    format!("{ENVELOPE_START}{header}<soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>")
}

const SESSION_HEADER: &str = r#"<soapenv:Header><ns1:sessionID soapenv:actor="http://schemas.xmlsoap.org/soap/actor/next" soapenv:mustUnderstand="0" xmlns:ns1="http://ws.polarion.com/session">-1234567</ns1:sessionID></soapenv:Header>"#;

fn xml(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "text/xml; charset=utf-8")
}

/// A work item the way Axis serializes it, with one custom field of each kind.
fn work_item_response() -> String {
    let t = r#"xmlns:t="http://ws.polarion.com/TrackerWebService-types""#;
    envelope(
        "",
        &format!(
            r#"<getWorkItemByIdResponse xmlns="http://ws.polarion.com/TrackerWebService-impl"><getWorkItemByIdReturn uri="subterra:data-service:objects:/default/PROJ${{WorkItem}}PROJ-1" unresolvable="false" {t}>
<t:customFields>
  <t:Custom><t:key>asil</t:key><t:value xsi:type="xsd:string">D</t:value></t:Custom>
  <t:Custom><t:key>severity2</t:key><t:value xsi:type="t:EnumOptionId"><t:id>major</t:id></t:value></t:Custom>
  <t:Custom><t:key>rationale</t:key><t:value xsi:type="b:Text" xmlns:b="http://ws.polarion.com/types"><b:type>text/html</b:type><b:content>&lt;p&gt;why&lt;/p&gt;</b:content><b:contentLossy>false</b:contentLossy></t:value></t:Custom>
  <t:Custom><t:key>platforms</t:key><t:value xsi:type="t:ArrayOfEnumOptionId"><t:EnumOptionId><t:id>a</t:id></t:EnumOptionId><t:EnumOptionId><t:id>b</t:id></t:EnumOptionId></t:value></t:Custom>
  <t:Custom><t:key>reviewed</t:key><t:value xsi:type="xsd:date">2024-01-31</t:value></t:Custom>
  <t:Custom><t:key>effort</t:key><t:value xsi:type="xsd:float">1.5</t:value></t:Custom>
  <t:Custom><t:key>empty</t:key><t:value xsi:nil="true"/></t:Custom>
  <t:Custom><t:key>exotic</t:key><t:value xsi:type="x:Mystery" xmlns:x="urn:example"><x:a>1</x:a></t:value></t:Custom>
</t:customFields>
<t:id>PROJ-1</t:id>
<t:title>Brake &amp; steer</t:title>
<t:type><t:id>requirement</t:id></t:type>
<t:linkedWorkItems><t:LinkedWorkItem><t:role><t:id>parent</t:id></t:role><t:suspect>false</t:suspect><t:workItemURI>subterra:x</t:workItemURI></t:LinkedWorkItem></t:linkedWorkItems>
</getWorkItemByIdReturn></getWorkItemByIdResponse>"#
        ),
    )
}

fn login_response() -> String {
    envelope(
        SESSION_HEADER,
        r#"<logInWithTokenResponse xmlns="http://ws.polarion.com/SessionWebService-impl"/>"#,
    )
}

async fn setup(read_only: bool) -> (MockServer, Client) {
    let server = MockServer::start().await;
    let client = Client::builder(
        &format!("{}/polarion", server.uri()),
        Auth::Token("secret-token".into()),
    )
    .read_only(read_only)
    .build()
    .unwrap();
    (server, client)
}

#[tokio::test]
async fn token_login_session_header_cookie_and_custom_fields() {
    let (server, client) = setup(false).await;
    Mock::given(method("POST"))
        .and(path("/polarion/ws/services/SessionWebService"))
        .and(header("content-type", "text/xml; charset=utf-8"))
        .and(header("soapaction", "\"\""))
        .and(body_string_contains("logInWithToken>"))
        .and(body_string_contains(">AccessToken<"))
        .and(body_string_contains(">secret-token<"))
        .respond_with(xml(login_response()).insert_header("set-cookie", "JSESSIONID=abc; Path=/"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/polarion/ws/services/TrackerWebService"))
        .and(header("cookie", "JSESSIONID=abc"))
        .and(body_string_contains(
            r#"<ses:sessionID xmlns:ses="http://ws.polarion.com/session">-1234567</ses:sessionID>"#,
        ))
        .and(body_string_contains("getWorkItemById>"))
        .and(body_string_contains(">PROJ-1<"))
        .respond_with(xml(work_item_response()))
        .expect(2)
        .mount(&server)
        .await;

    let item: WorkItem = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert_eq!(client.session_id().as_deref(), Some("-1234567"));
    assert_eq!(item.id.as_deref(), Some("PROJ-1"));
    assert_eq!(item.title.as_deref(), Some("Brake & steer"));
    assert_eq!(item.unresolvable, Some(false));
    assert!(item.uri.as_deref().unwrap().ends_with("PROJ-1"));
    assert_eq!(
        item.type_.as_ref().and_then(|t| t.id.as_deref()),
        Some("requirement")
    );
    let link = &item.linked_work_items.as_ref().unwrap()[0];
    assert_eq!(
        link.role.as_ref().and_then(|r| r.id.as_deref()),
        Some("parent")
    );

    let fields: Vec<(&str, Option<&AnyValue>)> = item
        .custom_fields
        .as_ref()
        .unwrap()
        .iter()
        .map(|c| (c.key.as_str(), c.value.as_ref()))
        .collect();
    assert!(matches!(fields[0], ("asil", Some(AnyValue::String(s))) if s == "D"));
    assert!(
        matches!(fields[1], ("severity2", Some(AnyValue::EnumOptionId(e))) if e.id.as_deref() == Some("major"))
    );
    assert!(
        matches!(fields[2], ("rationale", Some(AnyValue::Text(t))) if t.content == "<p>why</p>" && t.type_ == "text/html")
    );
    assert!(
        matches!(fields[3], ("platforms", Some(AnyValue::ArrayOfEnumOptionId(v))) if v.len() == 2 && v[1].id.as_deref() == Some("b"))
    );
    assert!(matches!(fields[4], ("reviewed", Some(AnyValue::Date(d))) if d == "2024-01-31"));
    assert!(matches!(fields[5], ("effort", Some(AnyValue::Float(f))) if *f == 1.5));
    assert!(matches!(fields[6], ("empty", None)));
    assert!(
        matches!(fields[7], ("exotic", Some(AnyValue::Unknown { xsi_type: Some(t), xml })) if t == "{urn:example}Mystery" && xml.contains("<x:a>1</x:a>")),
        "{:?}",
        fields[7]
    );

    // The session is reused: no second login.
    client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
}

#[tokio::test]
async fn password_login() {
    let server = MockServer::start().await;
    let client = Client::new(
        &format!("{}/polarion", server.uri()),
        Auth::Password {
            user: "jdoe".into(),
            password: "pw".into(),
        },
    )
    .unwrap();
    Mock::given(path("/polarion/ws/services/SessionWebService"))
        .and(body_string_contains("logIn>"))
        .and(body_string_contains(">jdoe<"))
        .respond_with(xml(envelope(
            SESSION_HEADER,
            r#"<logInResponse xmlns="http://ws.polarion.com/SessionWebService-impl"/>"#,
        )))
        .expect(1)
        .mount(&server)
        .await;
    client.login().await.unwrap();
    assert_eq!(client.session_id().as_deref(), Some("-1234567"));
}

#[tokio::test]
async fn faults_are_reported_with_their_message() {
    let (server, client) = setup(false).await;
    Mock::given(path("/polarion/ws/services/SessionWebService"))
        .respond_with(
            ResponseTemplate::new(500).set_body_raw(
                envelope(
                    "",
                    "<soapenv:Fault><faultcode>soapenv:Server.generalException</faultcode><faultstring>Authentication failed.; nested exception is: Your Username or Password is incorrect</faultstring></soapenv:Fault>",
                ),
                "text/xml",
            ),
        )
        .mount(&server)
        .await;

    let err = client
        .tracker()
        .get_work_item_by_id("P", "P-1")
        .await
        .unwrap_err();
    match err {
        Error::Fault { code, message, .. } => {
            assert_eq!(code, "soapenv:Server.generalException");
            assert!(message.starts_with("Authentication failed"), "{message}");
        }
        other => panic!("expected fault, got {other:?}"),
    }
}

#[tokio::test]
async fn non_soap_errors_keep_status_and_body() {
    let (server, client) = setup(false).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>Bad gateway</html"))
        .mount(&server)
        .await;
    let err = client.login().await.unwrap_err();
    assert!(
        matches!(&err, Error::Status { status, body } if status.as_u16() == 502 && body.contains("Bad gateway")),
        "{err:?}"
    );
}

#[tokio::test]
async fn read_only_client_refuses_writes_before_sending() {
    let (server, client) = setup(true).await;
    Mock::given(path("/polarion/ws/services/SessionWebService"))
        .respond_with(xml(login_response()))
        .mount(&server)
        .await;
    Mock::given(path("/polarion/ws/services/TrackerWebService"))
        .respond_with(xml(work_item_response()))
        .mount(&server)
        .await;

    let err = client
        .tracker()
        .create_work_item(&WorkItem::default())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::ReadOnly { operation } if operation == "TrackerWebService.createWorkItem")
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );

    // Reads still work (login + call).
    client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
