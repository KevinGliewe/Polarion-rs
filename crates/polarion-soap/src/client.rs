use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
///
/// When a call fails with a SOAP fault or an HTTP error, the client asks the
/// server whether the session is still valid (`hasSubject`). If it has
/// expired, the client logs in again and repeats the call once. A write is
/// repeated only after a SOAP fault (the server answered and refused it), and
/// only when the write filter lets it through again: after an HTTP error it may
/// have been applied, so its error is returned. Inside an
/// explicit transaction (`beginTransaction`) nothing is repeated: the transaction
/// is gone with the session, so the call fails with [`Error::SessionExpired`].
/// A [`ClientBuilder::transaction_only`] client never logs in a second time.
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

/// Decides whether a write may be sent ([`ClientBuilder::write_filter`]): called with the
/// operation name (`updateWorkItem`) and the request body (the operation element, without the
/// envelope).
pub type WriteFilter = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// A [`WriteFilter`] that can be printed.
#[derive(Clone)]
struct Filter(WriteFilter);

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteFilter(..)")
    }
}

#[derive(Debug)]
struct Inner {
    http: reqwest::Client,
    services: Url,
    auth: Auth,
    read_only: bool,
    write_filter: Option<Filter>,
    /// See [`ClientBuilder::transaction_only`].
    transaction_only: bool,
    session: Mutex<Option<String>>,
    /// Serializes logins, so concurrent calls never open more than one session.
    login_lock: tokio::sync::Mutex<()>,
    /// Incremented by every login; tells a failed call whether another call already logged in again.
    generation: AtomicU64,
    /// An explicit transaction is open in the current session.
    in_transaction: AtomicBool,
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
            write_filter: None,
            transaction_only: false,
        }
    }

    /// URL of the web services directory (always ends in `/ws/services/`).
    pub fn services_url(&self) -> &Url {
        &self.inner.services
    }

    pub fn is_read_only(&self) -> bool {
        self.inner.read_only
    }

    /// Whether writes are checked by a [`ClientBuilder::write_filter`].
    pub fn has_write_filter(&self) -> bool {
        self.inner.write_filter.is_some()
    }

    /// Whether an explicit transaction (`beginTransaction`) is open in the current session, as far
    /// as this client knows; nothing is sent. It is reset when the session ends or expires.
    pub fn transaction_open(&self) -> bool {
        self.inner.in_transaction.load(Ordering::SeqCst)
    }

    /// The current session id, if logged in.
    pub fn session_id(&self) -> Option<String> {
        self.inner.session.lock().expect("session lock").clone()
    }

    /// Log in now instead of on the first call (e.g. to fail fast on bad credentials).
    pub async fn login(&self) -> Result<()> {
        let _login = self.inner.login_lock.lock().await;
        self.log_in().await
    }

    /// Log in (callers hold `login_lock`; session operations never take it).
    async fn log_in(&self) -> Result<()> {
        self.clear_session();
        if self.inner.transaction_only && self.inner.generation.load(Ordering::SeqCst) > 0 {
            // The session, and the transaction in it, is gone; a new one would hold neither.
            return Err(Error::SessionExpired);
        }
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
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// End the session on the server. The local session (and transaction flag) is cleared even
    /// when `endSession` fails; its error is returned.
    pub async fn logout(&self) -> Result<()> {
        let _login = self.inner.login_lock.lock().await;
        let ended = match self.session_id() {
            Some(_) => self.session().end_session().await.map(drop),
            None => Ok(()),
        };
        self.clear_session();
        ended
    }

    fn clear_session(&self) {
        *self.inner.session.lock().expect("session lock") = None;
        self.inner.in_transaction.store(false, Ordering::SeqCst);
    }

    /// Log in unless another call did so while this one waited for the lock.
    async fn ensure_session(&self) -> Result<()> {
        let _login = self.inner.login_lock.lock().await;
        if self.session_id().is_none() {
            self.log_in().await?;
        }
        Ok(())
    }

    /// A call made in login `generation` (inside an explicit transaction or not) failed with a
    /// fault or HTTP error: log in again if the session has expired. `Ok(true)` means the session
    /// was renewed, so the call may be repeated.
    async fn renew(&self, generation: u64, in_transaction: bool) -> Result<bool> {
        let _login = self.inner.login_lock.lock().await;
        // Another call may have found the session expired and logged in again already.
        let renewed = self.inner.generation.load(Ordering::SeqCst) != generation;
        if !renewed {
            match self.session().has_subject().await {
                Ok(true) => return Ok(false),
                Ok(false) | Err(Error::Fault { .. }) => {}
                // The server is not answering properly; the original error says more.
                Err(_) => return Ok(false),
            }
        }
        if in_transaction {
            // The transaction ended with its session; repeating the call would apply it on its own.
            if !renewed {
                self.clear_session();
            }
            return Err(Error::SessionExpired);
        }
        if !renewed {
            self.log_in().await?;
        }
        Ok(true)
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
        if !read {
            if self.inner.read_only {
                return Err(Error::ReadOnly {
                    operation: format!("{service}.{operation}"),
                });
            }
            if !self.filter_allows(operation, &body) {
                return Err(Error::WriteFiltered {
                    operation: format!("{service}.{operation}"),
                });
            }
            if self.inner.transaction_only
                && !self.transaction_open()
                && !(service == SESSION_SERVICE && operation == "beginTransaction")
            {
                return Err(Error::NoTransaction {
                    operation: format!("{service}.{operation}"),
                });
            }
        }
        let needs_session = !(service == SESSION_SERVICE
            && matches!(
                operation,
                "logIn" | "logInWithToken" | "endSession" | "hasSubject"
            ));
        if needs_session && self.session_id().is_none() {
            Box::pin(self.ensure_session()).await?;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let in_transaction = self.inner.in_transaction.load(Ordering::SeqCst);
        // Taken by the attempt that succeeds; still there when the first one failed.
        let mut decode = Some(decode);
        match self.send(service, operation, &body, &mut decode).await {
            Err(e @ (Error::Fault { .. } | Error::Status { .. })) if needs_session => {
                let renewed = Box::pin(self.renew(generation, in_transaction)).await?;
                // A fault means the server answered and refused the call. After an HTTP status
                // error a write may have been applied, so it is not sent again. A repeated write
                // passes the write filter again: a filter that counts requests (a create
                // budget) counts the repeat, and may refuse it.
                let repeat = read || matches!(e, Error::Fault { .. });
                if renewed && repeat && (read || self.filter_allows(operation, &body)) {
                    self.send(service, operation, &body, &mut decode).await
                } else {
                    Err(e)
                }
            }
            result => result,
        }
    }

    /// Whether the write filter (if any) lets this write through.
    fn filter_allows(&self, operation: &str, body: &str) -> bool {
        self.inner
            .write_filter
            .as_ref()
            .is_none_or(|Filter(filter)| filter(operation, body))
    }

    /// One request: send the envelope, check for faults, decode the response.
    async fn send<T, D>(
        &self,
        service: &str,
        operation: &str,
        body: &str,
        decode: &mut Option<D>,
    ) -> Result<T>
    where
        D: FnOnce(Node<'_, '_>) -> Result<T>,
    {
        let envelope = envelope(self.session_id().as_deref(), body);
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
        let Some(body_node) = child(root, SOAP_ENV, "Body") else {
            // An error page that happens to be well-formed XML (proxies, gateways).
            if !status.is_success() {
                return Err(Error::Status { status, body: text });
            }
            return Err(Error::decode("Envelope", "no Body"));
        };
        let payload = body_node.children().find(|n| n.is_element());
        if let Some(fault) = payload.filter(|n| n.tag_name().name() == "Fault") {
            return Err(parse_fault(fault));
        }
        if status != StatusCode::OK {
            return Err(Error::Status { status, body: text });
        }
        if service == SESSION_SERVICE {
            match operation {
                "beginTransaction" => self.inner.in_transaction.store(true, Ordering::SeqCst),
                "endTransaction" => self.inner.in_transaction.store(false, Ordering::SeqCst),
                _ => {}
            }
        }
        let payload =
            payload.ok_or_else(|| Error::decode("Body", format!("no {operation}Response")))?;
        let decode = decode.take().expect("decoded at most once");
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

/// Builder for [`Client`]. A clone builds a client of its own, with its own session and cookies.
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: String,
    auth: Auth,
    timeout: Duration,
    accept_invalid_certs: bool,
    root_certificates: Vec<reqwest::Certificate>,
    read_only: bool,
    write_filter: Option<Filter>,
    transaction_only: bool,
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
    /// `has…`, `can…`, `is…`, `was…`, `export…`, plus `generateHistory` and
    /// login/logout. Everything else fails with [`Error::ReadOnly`].
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Check every operation that may modify data with `filter` before it is sent (after the
    /// read-only check, before logging in): `filter(operation, body)` gets the operation name
    /// (`updateWorkItem`) and the request body (the operation element, without the envelope),
    /// and `false` refuses the operation with [`Error::WriteFiltered`]. A write repeated after a
    /// session renewal is checked again, so a filter that counts requests counts the repeat too;
    /// when it refuses the repeat, the call fails with the original error.
    pub fn write_filter(mut self, filter: WriteFilter) -> Self {
        self.write_filter = Some(Filter(filter));
        self
    }

    /// Make a client for one transaction: it logs in at most once, so a second login (a session
    /// renewal included) fails with [`Error::SessionExpired`], and every write except
    /// `beginTransaction` is refused with [`Error::NoTransaction`] while no transaction is open
    /// (after the write filter, before anything is sent).
    pub fn transaction_only(mut self, transaction_only: bool) -> Self {
        self.transaction_only = transaction_only;
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
                write_filter: self.write_filter,
                transaction_only: self.transaction_only,
                session: Mutex::new(None),
                login_lock: tokio::sync::Mutex::new(()),
                generation: AtomicU64::new(0),
                in_transaction: AtomicBool::new(false),
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
