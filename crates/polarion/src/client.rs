use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use polarion_soap::Auth;

use crate::documents::Documents;
use crate::error::{Error, Result};
use crate::items::WorkItems;
use crate::projects::{Projects, Users};
use crate::schema::Schema;

/// Convenient access to a Polarion server, built on the SOAP web services.
///
/// Operations are grouped by area: [`Polarion::items`], [`Polarion::documents`],
/// [`Polarion::schema`], [`Polarion::projects`] and [`Polarion::users`].
///
/// **Read-only by default.** Writes must be enabled per project with
/// [`PolarionBuilder::allow_writes`]; every write method checks the target's
/// project against that list before anything is sent.
///
/// Cheap to clone; clones share the session, which is opened on the first call.
#[derive(Debug, Clone)]
pub struct Polarion {
    soap: polarion_soap::Client,
    writable: Arc<BTreeSet<String>>,
}

impl Polarion {
    /// A read-only client for `base_url` (e.g. `https://polarion.example.com/polarion`).
    pub fn new(base_url: &str, auth: Auth) -> Result<Self> {
        Self::builder(base_url, auth).build()
    }

    /// Configure TLS, timeouts and writes.
    pub fn builder(base_url: &str, auth: Auth) -> PolarionBuilder {
        PolarionBuilder {
            soap: polarion_soap::Client::builder(base_url, auth),
            writable: BTreeSet::new(),
        }
    }

    /// Wrap an existing SOAP client. Writes through this library are refused
    /// (the client's own read-only setting still applies to [`Polarion::soap`]).
    pub fn from_client(soap: polarion_soap::Client) -> Self {
        Self {
            soap,
            writable: Arc::default(),
        }
    }

    /// The underlying SOAP client, for operations this library does not cover.
    ///
    /// Calls made through it are **not** checked against the write allowlist.
    pub fn soap(&self) -> &polarion_soap::Client {
        &self.soap
    }

    /// Projects this client may write to.
    pub fn writable_projects(&self) -> impl Iterator<Item = &str> {
        self.writable.iter().map(String::as_str)
    }

    /// End the session on the server.
    pub async fn logout(&self) -> Result<()> {
        Ok(self.soap.logout().await?)
    }

    pub fn items(&self) -> WorkItems<'_> {
        WorkItems::new(self)
    }

    pub fn documents(&self) -> Documents<'_> {
        Documents::new(self)
    }

    pub fn schema(&self) -> Schema<'_> {
        Schema::new(self)
    }

    pub fn projects(&self) -> Projects<'_> {
        Projects::new(self)
    }

    pub fn users(&self) -> Users<'_> {
        Users::new(self)
    }

    /// Refuse a write unless `project` is in the allowlist. Every write method calls this first.
    pub(crate) fn check_write(&self, operation: &'static str, project: Option<&str>) -> Result<()> {
        match project {
            Some(p) if self.writable.contains(p) => Ok(()),
            _ => Err(Error::WriteNotAllowed {
                operation,
                project: project.map(str::to_owned),
            }),
        }
    }
}

/// Builder for [`Polarion`].
#[derive(Debug)]
pub struct PolarionBuilder {
    soap: polarion_soap::ClientBuilder,
    writable: BTreeSet<String>,
}

impl PolarionBuilder {
    /// Per-request timeout (default: 120 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.soap = self.soap.timeout(timeout);
        self
    }

    /// Trust an additional CA certificate (PEM). The OS trust store is used by default.
    pub fn add_root_certificate_pem(mut self, pem: &[u8]) -> Result<Self> {
        self.soap = self.soap.add_root_certificate_pem(pem)?;
        Ok(self)
    }

    /// Disable TLS certificate validation. Only for test instances with self-signed certificates.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.soap = self.soap.danger_accept_invalid_certs(accept);
        self
    }

    /// Allow writes to these projects (by id). Writes to any other project are
    /// refused before they are sent. Without this the client is read-only.
    pub fn allow_writes<I, S>(mut self, projects: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.writable.extend(
            projects
                .into_iter()
                .map(Into::into)
                .filter(|p| !p.is_empty()),
        );
        self
    }

    pub fn build(self) -> Result<Polarion> {
        let soap = self.soap.read_only(self.writable.is_empty()).build()?;
        Ok(Polarion {
            soap,
            writable: Arc::new(self.writable),
        })
    }
}

pub(crate) fn not_found(kind: &'static str, id: impl std::fmt::Display) -> Error {
    Error::NotFound {
        kind,
        id: id.to_string(),
    }
}
