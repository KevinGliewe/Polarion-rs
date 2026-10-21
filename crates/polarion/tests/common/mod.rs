//! Mock SOAP server helpers shared by the integration tests.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use polarion::{Auth, Polarion};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
pub const PROJECT: &str = "/polarion/ws/services/ProjectWebService";
pub const TESTS: &str = "/polarion/ws/services/TestManagementWebService";
pub const PLANNING: &str = "/polarion/ws/services/PlanningWebService";
pub const SECURITY: &str = "/polarion/ws/services/SecurityWebService";
pub const SESSION: &str = "/polarion/ws/services/SessionWebService";
pub const URI: &str = "subterra:data-service:objects:/default/";

pub fn envelope(header: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsd="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{header}<soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>"#
    )
}

pub fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(envelope("", body), "text/xml; charset=utf-8")
}

/// A mock server that accepts a token login.
pub async fn server() -> MockServer {
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
    server
}

/// A mock server that gives every login a session of its own: `-1`, `-2`, ... (also as the
/// `JSESSIONID` cookie), and answers `beginTransaction`, `endTransaction`, `endSession` and
/// `hasSubject` (`true`). Mocks mounted with the default priority take precedence.
pub async fn session_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(SESSION))
        .and(body_string_contains("logInWithToken>"))
        .respond_with(Logins(AtomicUsize::new(0)))
        .with_priority(10)
        .mount(&server)
        .await;
    for (operation, response) in [
        ("beginTransaction", "<beginTransactionResponse/>"),
        ("endTransaction", "<endTransactionResponse/>"),
        ("endSession", "<endSessionResponse/>"),
        (
            "hasSubject",
            "<hasSubjectResponse><hasSubjectReturn>true</hasSubjectReturn></hasSubjectResponse>",
        ),
    ] {
        Mock::given(path(SESSION))
            .and(body_string_contains(format!("{operation}>")))
            .respond_with(xml(response))
            .with_priority(10)
            .mount(&server)
            .await;
    }
    server
}

/// Answers the n-th login with the session `-n`.
struct Logins(AtomicUsize);

impl Respond for Logins {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let id = format!("-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1);
        ResponseTemplate::new(200)
            .set_body_raw(
                envelope(
                    &format!(
                        r#"<soapenv:Header><ns1:sessionID xmlns:ns1="http://ws.polarion.com/session">{id}</ns1:sessionID></soapenv:Header>"#
                    ),
                    "<logInWithTokenResponse/>",
                ),
                "text/xml",
            )
            .insert_header("set-cookie", format!("JSESSIONID={id}; Path=/").as_str())
    }
}

/// The requests received, in order, as (session id, operation); the session id is empty for
/// requests without one (logins).
pub async fn log(server: &MockServer) -> Vec<(String, String)> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|r| {
            let body = String::from_utf8(r.body).unwrap();
            let session = body
                .split_once("<ses:sessionID")
                .and_then(|(_, rest)| rest.split_once('>'))
                .and_then(|(_, rest)| rest.split_once("</ses:sessionID>"))
                .map_or("", |(id, _)| id)
                .to_owned();
            let element = body
                .split_once("<soapenv:Body><")
                .map_or("", |(_, rest)| rest)
                .split([' ', '>', '/'])
                .next()
                .unwrap_or_default();
            let operation = element.rsplit(':').next().unwrap_or_default().to_owned();
            (session, operation)
        })
        .collect()
}

/// A SOAP fault (HTTP 500).
pub fn fault(message: &str) -> ResponseTemplate {
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

/// The answer to `hasSubject`.
pub fn subject(valid: bool) -> ResponseTemplate {
    xml(&format!(
        "<hasSubjectResponse><hasSubjectReturn>{valid}</hasSubjectReturn></hasSubjectResponse>"
    ))
}

pub fn base_url(server: &MockServer) -> String {
    format!("{}/polarion", server.uri())
}

/// A client that may write to `Sandbox` only.
pub fn writer(server: &MockServer) -> Polarion {
    Polarion::builder(&base_url(server), Auth::Token("t".into()))
        .allow_writes(["Sandbox"])
        .build()
        .unwrap()
}

/// A client that may write to `Sandbox` and globally (users, global folders, ...).
pub fn admin(server: &MockServer) -> Polarion {
    Polarion::builder(&base_url(server), Auth::Token("t".into()))
        .allow_writes(["Sandbox"])
        .allow_global_writes(true)
        .build()
        .unwrap()
}

/// A read-only client.
pub fn reader(server: &MockServer) -> Polarion {
    Polarion::new(&base_url(server), Auth::Token("t".into())).unwrap()
}

/// Expect exactly one request to `service` whose body contains every needle.
pub async fn expect(server: &MockServer, service: &str, needles: &[&str], body: &str) {
    let mut m = Mock::given(path(service));
    for n in needles {
        m = m.and(body_string_contains(*n));
    }
    m.respond_with(xml(body)).expect(1).mount(server).await;
}

/// The bodies of the requests received for one operation, in order.
pub async fn bodies(server: &MockServer, operation: &str) -> Vec<String> {
    let needle = format!(":{operation}>");
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|r| String::from_utf8(r.body).unwrap())
        .filter(|b| b.contains(&needle))
        .collect()
}

/// Number of requests other than logins.
pub async fn calls(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| !r.url.path().ends_with("SessionWebService"))
        .count()
}
