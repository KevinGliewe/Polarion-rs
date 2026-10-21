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

// ---- session renewal

const SESSION: &str = "/polarion/ws/services/SessionWebService";
const TRACKER: &str = "/polarion/ws/services/TrackerWebService";

/// Logins answer with these session ids, one per login, in order (each expected exactly once).
async fn logins(server: &MockServer, ids: &[&str]) {
    for (i, id) in ids.iter().enumerate() {
        let header = format!(
            r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">{id}</ns1:sessionID></soapenv:Header>"#
        );
        Mock::given(path(SESSION))
            .and(body_string_contains("logInWithToken>"))
            .respond_with(xml(envelope(&header, "<logInWithTokenResponse/>")))
            .up_to_n_times(1)
            .with_priority(u8::try_from(i + 1).unwrap())
            .expect(1)
            .mount(server)
            .await;
    }
}

/// Requests sent with this session id.
fn session_is(id: &str) -> wiremock::matchers::BodyContainsMatcher {
    body_string_contains(format!(">{id}</ses:sessionID>"))
}

fn fault(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(500).set_body_raw(
        envelope(
            "",
            &format!(
                "<soapenv:Fault><faultcode>soapenv:Server.userException</faultcode><faultstring>{message}</faultstring></soapenv:Fault>"
            ),
        ),
        "text/xml",
    )
}

async fn has_subject(server: &MockServer, session: &str, answer: ResponseTemplate) {
    Mock::given(path(SESSION))
        .and(body_string_contains("hasSubject>"))
        .and(session_is(session))
        .respond_with(answer)
        .expect(1)
        .mount(server)
        .await;
}

fn subject(valid: bool) -> ResponseTemplate {
    xml(envelope(
        "",
        &format!(
            "<hasSubjectResponse><hasSubjectReturn>{valid}</hasSubjectReturn></hasSubjectResponse>"
        ),
    ))
}

async fn get_item(server: &MockServer, session: &str, answer: ResponseTemplate, times: u64) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemById>"))
        .and(session_is(session))
        .respond_with(answer)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn expired_sessions_are_renewed() {
    let (server, client) = setup(true).await;
    logins(&server, &["s1", "s2"]).await;
    get_item(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(false)).await;
    get_item(&server, "s2", xml(work_item_response()), 1).await;

    let item = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert_eq!(item.id.as_deref(), Some("PROJ-1"));
    assert_eq!(client.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn faults_with_a_live_session_are_not_retried() {
    let (server, client) = setup(true).await;
    logins(&server, &["s1"]).await;
    get_item(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(true)).await;

    let err = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Fault { message, .. } if message == "Not authorized."),
        "{err:?}"
    );
}

#[tokio::test]
async fn server_errors_are_not_mistaken_for_expiry() {
    let (server, client) = setup(true).await;
    logins(&server, &["s1"]).await;
    let bad_gateway = || ResponseTemplate::new(502).set_body_string("<html>Bad gateway</html>");
    get_item(&server, "s1", bad_gateway(), 1).await;
    has_subject(&server, "s1", bad_gateway()).await;

    let err = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Status { status, .. } if status.as_u16() == 502),
        "{err:?}"
    );
}

#[tokio::test]
async fn expiry_inside_a_transaction_is_reported() {
    let (server, client) = setup(false).await;
    logins(&server, &["s1", "s2"]).await;
    Mock::given(path(SESSION))
        .and(body_string_contains("beginTransaction>"))
        .and(session_is("s1"))
        .respond_with(xml(envelope("", "<beginTransactionResponse/>")))
        .expect(1)
        .mount(&server)
        .await;
    get_item(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(false)).await;
    get_item(&server, "s2", xml(work_item_response()), 1).await;

    // A session operation as the first call logs in, too.
    client.session().begin_transaction().await.unwrap();
    let err = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::SessionExpired), "{err:?}");
    assert_eq!(client.session_id(), None);

    // The next call starts a new session.
    client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert_eq!(client.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn concurrent_first_calls_log_in_once() {
    let (server, client) = setup(true).await;
    Mock::given(path(SESSION))
        .and(body_string_contains("logInWithToken>"))
        .respond_with(xml(login_response()).set_delay(std::time::Duration::from_millis(200)))
        .expect(1)
        .mount(&server)
        .await;
    get_item(&server, "-1234567", xml(work_item_response()), 2).await;

    let tracker = client.tracker();
    let (a, b) = tokio::join!(
        tracker.get_work_item_by_id("PROJ", "PROJ-1"),
        tracker.get_work_item_by_id("PROJ", "PROJ-1"),
    );
    a.unwrap();
    b.unwrap();
}

// ---- writes after session renewal

const ITEM_URI: &str = "subterra:data-service:objects:/default/PROJ${WorkItem}PROJ-1";

fn role() -> polarion_soap::types::EnumOptionId {
    polarion_soap::types::EnumOptionId {
        id: Some("ref_ext".into()),
    }
}

/// A write (`addHyperlink`) sent with this session, answered with `answer`, expected `times` times.
async fn add_hyperlink(server: &MockServer, session: &str, answer: ResponseTemplate, times: u64) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("addHyperlink>"))
        .and(session_is(session))
        .respond_with(answer)
        .expect(times)
        .mount(server)
        .await;
}

fn hyperlink_added() -> ResponseTemplate {
    xml(envelope(
        "",
        "<addHyperlinkResponse><addHyperlinkReturn>true</addHyperlinkReturn></addHyperlinkResponse>",
    ))
}

#[tokio::test]
async fn writes_are_repeated_after_a_fault() {
    let (server, client) = setup(false).await;
    logins(&server, &["s1", "s2"]).await;
    add_hyperlink(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(false)).await;
    add_hyperlink(&server, "s2", hyperlink_added(), 1).await;

    // The server answered and refused the write, so it was not applied: it is sent again.
    let added = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
        .await
        .unwrap();
    assert!(added);
    assert_eq!(client.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn writes_are_not_repeated_after_an_http_error() {
    let (server, client) = setup(false).await;
    logins(&server, &["s1", "s2"]).await;
    let bad_gateway = ResponseTemplate::new(502).set_body_string("<html>Bad gateway</html>");
    add_hyperlink(&server, "s1", bad_gateway, 1).await;
    has_subject(&server, "s1", subject(false)).await;
    add_hyperlink(&server, "s2", hyperlink_added(), 0).await;

    // The write may have been applied: its error is returned, and the session is renewed for
    // the next call.
    let err = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Status { status, .. } if status.as_u16() == 502),
        "{err:?}"
    );
    assert_eq!(client.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn repeated_writes_pass_the_write_filter_again() {
    // A filter that lets one request through, like a create budget of one.
    let server = MockServer::start().await;
    let left = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let budget = left.clone();
    let client = Client::builder(
        &format!("{}/polarion", server.uri()),
        Auth::Token("secret-token".into()),
    )
    .write_filter(std::sync::Arc::new(move |_: &str, _: &str| {
        budget.swap(false, std::sync::atomic::Ordering::SeqCst)
    }))
    .build()
    .unwrap();
    logins(&server, &["s1", "s2"]).await;
    add_hyperlink(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(false)).await;
    add_hyperlink(&server, "s2", hyperlink_added(), 0).await;

    // The repeat after the renewal is refused by the filter: the fault is returned, and only
    // one request was sent.
    let err = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Fault { message, .. } if message == "Not authorized."),
        "{err:?}"
    );
    assert!(!left.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(client.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn reads_are_repeated_after_an_http_error() {
    let (server, client) = setup(true).await;
    logins(&server, &["s1", "s2"]).await;
    let bad_gateway = ResponseTemplate::new(502).set_body_string("<html>Bad gateway</html>");
    get_item(&server, "s1", bad_gateway, 1).await;
    has_subject(&server, "s1", subject(false)).await;
    get_item(&server, "s2", xml(work_item_response()), 1).await;

    let item = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert_eq!(item.id.as_deref(), Some("PROJ-1"));
}

// ---- write filter

#[tokio::test]
async fn the_write_filter_sees_every_write_before_it_is_sent() {
    let server = MockServer::start().await;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let log = seen.clone();
    let client = Client::builder(
        &format!("{}/polarion", server.uri()),
        Auth::Token("secret-token".into()),
    )
    .write_filter(std::sync::Arc::new(move |operation: &str, body: &str| {
        log.lock()
            .unwrap()
            .push((operation.to_owned(), body.to_owned()));
        !body.contains("forbidden")
    }))
    .build()
    .unwrap();
    assert!(client.has_write_filter() && !client.is_read_only());
    Mock::given(path(SESSION))
        .respond_with(xml(login_response()))
        .mount(&server)
        .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("addHyperlink>"))
        .respond_with(hyperlink_added())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemById>"))
        .respond_with(xml(work_item_response()))
        .mount(&server)
        .await;

    let err = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid/forbidden", &role())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::WriteFiltered { operation } if operation == "TrackerWebService.addHyperlink"),
        "{err:?}"
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );

    // Reads are not filtered; allowed writes are sent.
    client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap();
    assert!(
        client
            .tracker()
            .add_hyperlink(ITEM_URI, "https://example.invalid/ok", &role())
            .await
            .unwrap()
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "only the writes: {seen:?}");
    assert_eq!(seen[1].0, "addHyperlink");
    assert!(
        seen[1].1.starts_with("<trackeri:addHyperlink>")
            && seen[1].1.contains(&format!("workitemURI>{ITEM_URI}<")),
        "the body without the envelope: {}",
        seen[1].1
    );
}

// ---- separate sessions and transaction-only clients

/// Logins answer with these session ids and a cookie named after each, one per login, in order.
async fn logins_with_cookies(server: &MockServer, ids: &[&str]) {
    for (i, id) in ids.iter().enumerate() {
        let header = format!(
            r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">{id}</ns1:sessionID></soapenv:Header>"#
        );
        Mock::given(path(SESSION))
            .and(body_string_contains("logInWithToken>"))
            .respond_with(
                xml(envelope(&header, "<logInWithTokenResponse/>"))
                    .insert_header("set-cookie", format!("JSESSIONID={id}; Path=/").as_str()),
            )
            .up_to_n_times(1)
            .with_priority(u8::try_from(i + 1).unwrap())
            .expect(1)
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn clients_from_a_cloned_builder_have_sessions_of_their_own() {
    let server = MockServer::start().await;
    let builder = Client::builder(
        &format!("{}/polarion", server.uri()),
        Auth::Token("secret-token".into()),
    )
    .read_only(true);
    let first = builder.clone().build().unwrap();
    let second = builder.build().unwrap();
    logins_with_cookies(&server, &["s1", "s2"]).await;
    for (session, times) in [("s1", 2), ("s2", 1)] {
        Mock::given(path(TRACKER))
            .and(body_string_contains("getWorkItemById>"))
            .and(session_is(session))
            .and(header("cookie", format!("JSESSIONID={session}").as_str()))
            .respond_with(xml(work_item_response()))
            .expect(times)
            .mount(&server)
            .await;
    }

    // The second login sets its cookie in the second client's jar only.
    for client in [&first, &second, &first] {
        client
            .tracker()
            .get_work_item_by_id("PROJ", "PROJ-1")
            .await
            .unwrap();
    }
    assert_eq!(first.session_id().as_deref(), Some("s1"));
    assert_eq!(second.session_id().as_deref(), Some("s2"));
}

#[tokio::test]
async fn transaction_only_clients_write_inside_their_transaction_and_log_in_once() {
    let server = MockServer::start().await;
    let client = Client::builder(
        &format!("{}/polarion", server.uri()),
        Auth::Token("secret-token".into()),
    )
    .transaction_only(true)
    .build()
    .unwrap();

    // Before the transaction: refused, nothing sent (not even a login).
    let err = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::NoTransaction { operation } if operation == "TrackerWebService.addHyperlink"),
        "{err:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());

    logins(&server, &["s1"]).await;
    for operation in ["beginTransaction", "endTransaction"] {
        Mock::given(path(SESSION))
            .and(body_string_contains(format!("{operation}>")))
            .and(session_is("s1"))
            .respond_with(xml(envelope("", &format!("<{operation}Response/>"))))
            .expect(1)
            .mount(&server)
            .await;
    }
    add_hyperlink(&server, "s1", hyperlink_added(), 1).await;
    get_item(&server, "s1", fault("Not authorized."), 1).await;
    has_subject(&server, "s1", subject(false)).await;

    // The transaction's begin is the only login.
    client.session().begin_transaction().await.unwrap();
    assert!(client.transaction_open());
    assert!(
        client
            .tracker()
            .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
            .await
            .unwrap()
    );
    client.session().end_transaction(false).await.unwrap();
    assert!(!client.transaction_open());

    // After the commit, writes are refused again.
    let err = client
        .tracker()
        .add_hyperlink(ITEM_URI, "https://example.invalid", &role())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NoTransaction { .. }), "{err:?}");

    // An expired session is not renewed, and no later call logs in again.
    let sent = server.received_requests().await.unwrap().len();
    let err = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::SessionExpired), "{err:?}");
    assert_eq!(client.session_id(), None);
    let err = client
        .tracker()
        .get_work_item_by_id("PROJ", "PROJ-1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::SessionExpired), "{err:?}");
    // The failed read and its hasSubject check; no further login.
    assert_eq!(server.received_requests().await.unwrap().len(), sent + 2);
}

#[tokio::test]
async fn logout_clears_the_session_when_end_session_fails() {
    let (server, client) = setup(false).await;
    logins(&server, &["s1"]).await;
    Mock::given(path(SESSION))
        .and(body_string_contains("beginTransaction>"))
        .respond_with(xml(envelope("", "<beginTransactionResponse/>")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(SESSION))
        .and(body_string_contains("endSession>"))
        .respond_with(ResponseTemplate::new(500).set_body_string("<html>Internal error</html>"))
        .expect(1)
        .mount(&server)
        .await;

    client.session().begin_transaction().await.unwrap();
    assert!(client.transaction_open());
    let err = client.logout().await.unwrap_err();
    assert!(
        matches!(&err, Error::Status { status, .. } if status.as_u16() == 500),
        "{err:?}"
    );
    assert_eq!(client.session_id(), None);
    assert!(!client.transaction_open());
}
