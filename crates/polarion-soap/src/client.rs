use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use roxmltree::{Document, Node};
use url::Url;

use crate::error::{Error, Result};
use crate::generated::NAMESPACES;
use crate::xml::{self, XSD, XSI};

const SOAP_ENV: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const SESSION_NS: &str = "http://ws.polarion.com/session";
const SESSION_SERVICE: &str = "SessionWebService";
const USER_AGENT: &str = concat!("polarion-rs/", env!("CARGO_PKG_VERSION"));
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// How the client logs in.
#[derive(Clone)]
pub enum Auth {
    /// Personal access token (`logInWithToken`, mechanism `AccessToken`).
    Token(String),
    /// User name and password (`logIn`).
    Password { user: String, password: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Token(_) => f.write_str("Token(..)"),
            Auth::Password { user, .. } => write!(f, "Password {{ user: {user:?}, .. }}"),
        }
    }
}

/// Async client for the Polarion SOAP web services.
///
/// Services are reached through accessors named after them
/// (`client.tracker()`, `client.project()`, ...); every WSDL operation is a
/// method in snake case (`queryWorkItems` → `query_work_items`).
///
/// The client logs in lazily on the first call and keeps the session (the
/// `sessionID` SOAP header plus cookies) for all services. Cheap to clone;
/// clones share the session.
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    http: reqwest::Client,
    services: Url,
    auth: Auth,
    read_only: bool,
    session: Mutex<Option<String>>,
}

impl Client {
    /// Create a client with default settings.
    ///
    /// `base_url` is the Polarion root, e.g. `https://polarion.example.com/polarion`;
    /// `ws/services/` is appended unless already present.
    pub fn new(base_url: &str, auth: Auth) -> Result<Self> {
        Self::builder(base_url, auth).build()
    }

    pub fn builder(base_url: &str, auth: Auth) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.to_owned(),
            auth,
            timeout: DEFAULT_TIMEOUT,
            accept_invalid_certs: false,
            root_certificates: Vec::new(),
            read_only: false,
        }
    }

    /// URL of the web services directory (always ends in `/ws/services/`).
    pub fn services_url(&self) -> &Url {
        &self.inner.services
    }

    pub fn is_read_only(&self) -> bool {
        self.inner.read_only
    }

    /// The current session id, if logged in.
    pub fn session_id(&self) -> Option<String> {
        self.inner.session.lock().expect("session lock").clone()
    }

    /// Log in now instead of on the first call (e.g. to fail fast on bad credentials).
    pub async fn login(&self) -> Result<()> {
        self.clear_session();
        match &self.inner.auth {
            Auth::Token(token) => {
                self.session()
                    .log_in_with_token("AccessToken", "", token)
                    .await?
            }
            Auth::Password { user, password } => self.session().log_in(user, password).await?,
        }
        if self.session_id().is_none() {
            return Err(Error::Login(
                "the server did not return a sessionID header".into(),
            ));
        }
        Ok(())
    }

    /// End the session on the server.
    pub async fn logout(&self) -> Result<()> {
        if self.session_id().is_some() {
            self.session().end_session().await?;
            self.clear_session();
        }
        Ok(())
    }

    fn clear_session(&self) {
        *self.inner.session.lock().expect("session lock") = None;
    }

    /// Send one operation and decode the response wrapper element with `decode`.
    pub(crate) async fn call<T>(
        &self,
        service: &str,
        operation: &str,
        read: bool,
        body: String,
        decode: impl FnOnce(Node<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        if self.inner.read_only && !read {
            return Err(Error::ReadOnly {
                operation: format!("{service}.{operation}"),
            });
        }
        if service != SESSION_SERVICE && self.session_id().is_none() {
            Box::pin(self.login()).await?;
        }

        let envelope = envelope(self.session_id().as_deref(), &body);
        let url = self.inner.services.join(service)?;
        let resp = self
            .inner
            .http
            .post(url)
            .header(
                CONTENT_TYPE,
                HeaderValue::from_static("text/xml; charset=utf-8"),
            )
            .header("SOAPAction", HeaderValue::from_static("\"\""))
            .body(envelope)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;

        let doc = match Document::parse(&text) {
            Ok(doc) => doc,
            Err(_) if !status.is_success() => return Err(Error::Status { status, body: text }),
            Err(e) => return Err(e.into()),
        };
        let root = doc.root_element();
        if let Some(id) = session_header(root) {
            *self.inner.session.lock().expect("session lock") = Some(id);
        }
        let body_node =
            child(root, SOAP_ENV, "Body").ok_or_else(|| Error::decode("Envelope", "no Body"))?;
        let payload = body_node.children().find(|n| n.is_element());
        if let Some(fault) = payload.filter(|n| n.tag_name().name() == "Fault") {
            return Err(parse_fault(fault));
        }
        if status != StatusCode::OK {
            return Err(Error::Status { status, body: text });
        }
        let payload =
            payload.ok_or_else(|| Error::decode("Body", format!("no {operation}Response")))?;
        decode(payload)
    }
}

fn envelope(session: Option<&str>, body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 2048);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push_str(r#"<soapenv:Envelope xmlns:soapenv=""#);
    out.push_str(SOAP_ENV);
    out.push_str(r#"" xmlns:xsi=""#);
    out.push_str(XSI);
    out.push_str(r#"" xmlns:xsd=""#);
    out.push_str(XSD);
    out.push('"');
    for (prefix, uri) in NAMESPACES {
        out.push_str(" xmlns:");
        out.push_str(prefix);
        out.push_str("=\"");
        out.push_str(uri);
        out.push('"');
    }
    out.push('>');
    if let Some(id) = session {
        let mut w = xml::XmlWriter::new();
        w.open("ses", "sessionID");
        w.attr("xmlns:ses", SESSION_NS);
        w.open_end();
        w.text(id);
        w.close("ses", "sessionID");
        out.push_str("<soapenv:Header>");
        out.push_str(&w.into_string());
        out.push_str("</soapenv:Header>");
    }
    out.push_str("<soapenv:Body>");
    out.push_str(body);
    out.push_str("</soapenv:Body></soapenv:Envelope>");
    out
}

fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| {
        c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(ns)
    })
}

fn session_header(envelope: Node<'_, '_>) -> Option<String> {
    let header = child(envelope, SOAP_ENV, "Header")?;
    let id = child(header, SESSION_NS, "sessionID")?;
    let id = xml::text(id);
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

fn parse_fault(fault: Node<'_, '_>) -> Error {
    let field = |name: &str| {
        fault
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == name)
    };
    Error::Fault {
        code: field("faultcode")
            .map(xml::text)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        message: field("faultstring")
            .map(xml::text)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        detail: field("detail").map(xml::raw_xml),
    }
}

/// Builder for [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    base_url: String,
    auth: Auth,
    timeout: Duration,
    accept_invalid_certs: bool,
    root_certificates: Vec<reqwest::Certificate>,
    read_only: bool,
}

impl ClientBuilder {
    /// Per-request timeout (default: 120 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Trust an additional CA certificate (PEM). The OS trust store is used by default.
    pub fn add_root_certificate_pem(mut self, pem: &[u8]) -> Result<Self> {
        self.root_certificates
            .push(reqwest::Certificate::from_pem(pem)?);
        Ok(self)
    }

    /// Disable TLS certificate validation. Only for test instances with self-signed certificates.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Refuse every operation that may modify data, before it is sent.
    ///
    /// Allowed: operations named `get…`, `query…`, `list…`, `search…`,
    /// `has…`, `can…`, `is…`, `was…`, `export…` and login/logout. Everything
    /// else fails with [`Error::ReadOnly`].
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn build(self) -> Result<Client> {
        let services = normalize_base_url(&self.base_url)?;
        let mut http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .cookie_store(true)
            .timeout(self.timeout)
            .tls_danger_accept_invalid_certs(self.accept_invalid_certs);
        for cert in self.root_certificates {
            http = http.add_root_certificate(cert);
        }
        Ok(Client {
            inner: Arc::new(Inner {
                http: http.build()?,
                services,
                auth: self.auth,
                read_only: self.read_only,
                session: Mutex::new(None),
            }),
        })
    }
}

/// Parse the user-supplied URL and make it point at `<root>/ws/services/`.
fn normalize_base_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input.trim())?;
    if url.cannot_be_a_base() {
        return Err(Error::Config(format!("not a base URL: {input}")));
    }
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let has_suffix = segments.len() >= 2 && segments[segments.len() - 2..] == ["ws", "services"];
    {
        let mut path = url.path_segments_mut().expect("checked cannot_be_a_base");
        path.clear().extend(&segments);
        if !has_suffix {
            path.extend(["ws", "services"]);
        }
        path.push("");
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_normalization() {
        for (input, expected) in [
            (
                "https://host/polarion",
                "https://host/polarion/ws/services/",
            ),
            (
                "https://host/polarion/",
                "https://host/polarion/ws/services/",
            ),
            (
                "https://host/polarion/ws/services",
                "https://host/polarion/ws/services/",
            ),
            (
                "https://host/polarion/ws/services/",
                "https://host/polarion/ws/services/",
            ),
        ] {
            assert_eq!(
                normalize_base_url(input).unwrap().as_str(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn auth_debug_hides_secrets() {
        let token = format!("{:?}", Auth::Token("secret".into()));
        let password = format!(
            "{:?}",
            Auth::Password {
                user: "u".into(),
                password: "secret".into()
            }
        );
        assert!(!token.contains("secret") && !password.contains("secret"));
    }
}
