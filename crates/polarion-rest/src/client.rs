use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::multipart::Part;
use reqwest::{Method, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

use crate::error::{Error, Result};
use crate::jsonapi::{ErrorDocument, ListEnvelope};
use crate::query::Query;

const USER_AGENT: &str = concat!("polarion-rs/", env!("CARGO_PKG_VERSION"));
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const REST_PATH: [&str; 2] = ["rest", "v1"];

/// Async client for the Polarion REST API.
///
/// Every operation of the API is available as a method named after its
/// `operationId` in snake case (`getWorkItems` → [`Client::get_work_items`]).
/// Cheap to clone; clones share the underlying connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    read_only: bool,
}

impl Client {
    /// Create a client with default settings.
    ///
    /// `base_url` is the Polarion root, e.g. `https://polarion.example.com/polarion`.
    /// The `rest/v1` suffix is appended unless already present.
    /// `token` is a Polarion personal access token.
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        Self::builder(base_url, token).build()
    }

    pub fn builder(base_url: &str, token: &str) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.to_owned(),
            token: token.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            accept_invalid_certs: false,
            root_certificates: Vec::new(),
            read_only: false,
        }
    }

    /// Whether this client refuses everything but `GET` requests.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Base URL of the REST API (always ends in `/rest/v1/`).
    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Build an endpoint URL from path segments. Each segment is percent-encoded,
    /// so ids containing spaces or other reserved characters are safe.
    pub fn url<S: AsRef<str>>(&self, segments: &[S]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("base URL is validated in ClientBuilder::build")
            .pop_if_empty()
            .extend(segments.iter().map(AsRef::as_ref));
        url
    }

    /// Call any endpoint with an optional JSON body.
    ///
    /// Escape hatch for raw access. `path` is relative to the REST base and
    /// split on `/` into segments, e.g. `"projects/MyProject/workitems"`.
    /// Returns `None` for empty responses (e.g. `204 No Content`).
    pub async fn request_json(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut req = self.request(method, &segments, Some(query));
        if let Some(body) = body {
            req = req.json(body);
        }
        let text = self.execute(req).await?.text().await?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| Error::Decode { source, body: text })
    }

    /// Fetch every page of a list endpoint and merge the results.
    ///
    /// `fetch` is called with `query` plus `page[number]` (and `page[size]`,
    /// defaulting to 100 if unset) until the server stops reporting a next page.
    ///
    /// ```no_run
    /// # async fn f(client: polarion_rest::Client) -> polarion_rest::Result<()> {
    /// use polarion_rest::Query;
    ///
    /// let all = client
    ///     .paginate(Query::new().query("type:task"), async |q| {
    ///         client.get_work_items("MyProject", &q).await
    ///     })
    ///     .await?;
    /// println!("{} work items", all.data.len());
    /// # Ok(()) }
    /// ```
    pub async fn paginate<A, F>(&self, query: Query, mut fetch: F) -> Result<ListEnvelope<A>>
    where
        F: AsyncFnMut(Query) -> Result<ListEnvelope<A>>,
    {
        let page_size: u32 = query
            .get("page[size]")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);
        let mut page: u32 = query
            .get("page[number]")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let query = query.page_size(page_size);

        let mut merged: Option<ListEnvelope<A>> = None;
        loop {
            let doc = fetch(query.clone().page_number(page)).await?;
            let done = !doc.has_next_page() || doc.data.len() < page_size as usize;
            merged = Some(match merged {
                None => doc,
                Some(mut acc) => {
                    acc.data.extend(doc.data);
                    acc.included.extend(doc.included);
                    acc
                }
            });
            if done {
                break;
            }
            page += 1;
        }
        let mut merged = merged.expect("loop runs at least once");
        merged.links = None;
        Ok(merged)
    }

    // ---- plumbing used by the generated operations ----

    pub(crate) fn request<S: AsRef<str>>(
        &self,
        method: Method,
        segments: &[S],
        query: Option<&Query>,
    ) -> RequestBuilder {
        let mut req = self.http.request(method, self.url(segments));
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            req = req.query(query.pairs());
        }
        req
    }

    pub(crate) async fn execute_json<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let body = self.execute(req).await?.text().await?;
        serde_json::from_str(&body).map_err(|source| Error::Decode { source, body })
    }

    pub(crate) async fn execute_empty(&self, req: RequestBuilder) -> Result<()> {
        self.execute(req).await?;
        Ok(())
    }

    pub(crate) async fn execute_bytes(&self, req: RequestBuilder) -> Result<Vec<u8>> {
        Ok(self.execute(req).await?.bytes().await?.to_vec())
    }

    async fn execute(&self, req: RequestBuilder) -> Result<Response> {
        let req = req.build()?;
        if self.read_only && req.method() != Method::GET {
            return Err(Error::ReadOnly {
                method: req.method().clone(),
                url: req.url().to_string(),
            });
        }
        let resp = self.http.execute(req).await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        let errors = serde_json::from_str::<ErrorDocument>(&body)
            .map(|d| d.errors)
            .unwrap_or_default();
        Err(Error::Api {
            status,
            errors,
            body,
        })
    }
}

/// A file to upload in a multipart request (attachments, Word import, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePart {
    pub file_name: String,
    pub content: Vec<u8>,
    /// MIME type; defaults to `application/octet-stream`.
    pub mime_type: Option<String>,
}

impl FilePart {
    pub fn new(file_name: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            file_name: file_name.into(),
            content: content.into(),
            mime_type: None,
        }
    }

    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    pub(crate) fn into_part(self) -> Result<Part> {
        let part = Part::bytes(self.content).file_name(self.file_name);
        match self.mime_type {
            Some(mime) => Ok(part.mime_str(&mime)?),
            None => Ok(part),
        }
    }
}

/// Serialize `value` as an `application/json` multipart part.
pub(crate) fn json_part<T: Serialize + ?Sized>(value: &T) -> Result<Part> {
    let json = serde_json::to_vec(value).map_err(Error::Encode)?;
    Ok(Part::bytes(json).mime_str("application/json")?)
}

/// Builder for [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    base_url: String,
    token: String,
    timeout: Duration,
    accept_invalid_certs: bool,
    root_certificates: Vec<reqwest::Certificate>,
    read_only: bool,
}

impl ClientBuilder {
    /// Per-request timeout (default: 60 s).
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

    /// Refuse every request except `GET`, before it is sent.
    ///
    /// A safety net for exploring production servers: only reads reach the
    /// network, writes fail with [`Error::ReadOnly`]. Non-mutating `POST`
    /// operations are refused too.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn build(self) -> Result<Client> {
        let base = normalize_base_url(&self.base_url)?;

        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.token.trim()))
            .map_err(|_| Error::Config("token contains invalid header characters".into()))?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let mut http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .timeout(self.timeout)
            .tls_danger_accept_invalid_certs(self.accept_invalid_certs);
        for cert in self.root_certificates {
            http = http.add_root_certificate(cert);
        }

        Ok(Client {
            http: http.build()?,
            base,
            read_only: self.read_only,
        })
    }
}

/// Parse the user-supplied URL and make it point at `<root>/rest/v1/`.
fn normalize_base_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input.trim())?;
    if url.cannot_be_a_base() {
        return Err(Error::Config(format!("not a base URL: {input}")));
    }
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let has_suffix = segments.len() >= 2 && segments[segments.len() - 2..] == REST_PATH;
    {
        let mut path = url.path_segments_mut().expect("checked cannot_be_a_base");
        path.clear().extend(&segments);
        if !has_suffix {
            path.extend(REST_PATH);
        }
        // Trailing slash so that path extension behaves predictably.
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
            ("https://host/polarion", "https://host/polarion/rest/v1/"),
            ("https://host/polarion/", "https://host/polarion/rest/v1/"),
            (
                "https://host/polarion/rest/v1",
                "https://host/polarion/rest/v1/",
            ),
            (
                "https://host/polarion/rest/v1/",
                "https://host/polarion/rest/v1/",
            ),
            ("https://host", "https://host/rest/v1/"),
        ] {
            assert_eq!(
                normalize_base_url(input).unwrap().as_str(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn url_encodes_segments() {
        let c = Client::new("https://host/polarion", "t").unwrap();
        assert_eq!(
            c.url(&[
                "projects",
                "P",
                "spaces",
                "_default",
                "documents",
                "My Doc/x"
            ])
            .as_str(),
            "https://host/polarion/rest/v1/projects/P/spaces/_default/documents/My%20Doc%2Fx"
        );
    }
}
