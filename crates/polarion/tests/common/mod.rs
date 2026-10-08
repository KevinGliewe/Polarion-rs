//! Mock SOAP server helpers shared by the integration tests.

#![allow(dead_code)]

use polarion::{Auth, Polarion};
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const TRACKER: &str = "/polarion/ws/services/TrackerWebService";
pub const PROJECT: &str = "/polarion/ws/services/ProjectWebService";
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
    Mock::given(path("/polarion/ws/services/SessionWebService"))
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
