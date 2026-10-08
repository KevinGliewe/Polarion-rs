//! Connection settings from the environment (and command-line overrides).

use polarion::{Auth, Polarion};

use crate::error::{CommandError, Result};

/// Everything needed to connect, usually from environment variables:
///
/// | Variable | Meaning |
/// |---|---|
/// | `POLARION_URL` | Polarion base URL, e.g. `https://polarion.example.com/polarion` |
/// | `POLARION_TOKEN` | personal access token, or |
/// | `POLARION_USER`, `POLARION_PASSWORD` | user name and password |
/// | `POLARION_PROJECT` | default project for ids without a project |
/// | `POLARION_INSECURE=1` | accept invalid TLS certificates (test servers only) |
/// | `POLARION_WRITE_PROJECTS` | comma-separated projects that may be written; none: read-only |
/// | `POLARION_ALLOW_GLOBAL_WRITES=1` | allow writes not tied to a project (users, global folders, ...) |
#[derive(Clone, Default)]
pub struct Config {
    pub url: Option<String>,
    pub auth: Option<Auth>,
    pub project: Option<String>,
    pub insecure: bool,
    pub write_projects: Vec<String>,
    pub global_writes: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("auth", &self.auth)
            .field("project", &self.project)
            .field("insecure", &self.insecure)
            .field("write_projects", &self.write_projects)
            .field("global_writes", &self.global_writes)
            .finish()
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn flag(name: &str) -> bool {
    var(name).is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
}

/// Split a comma-separated list, dropping empty entries.
pub fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Config {
    pub fn from_env() -> Self {
        let auth = var("POLARION_TOKEN").map(Auth::Token).or_else(|| {
            Some(Auth::Password {
                user: var("POLARION_USER")?,
                password: var("POLARION_PASSWORD")?,
            })
        });
        Self {
            url: var("POLARION_URL"),
            auth,
            project: var("POLARION_PROJECT"),
            insecure: flag("POLARION_INSECURE"),
            write_projects: var("POLARION_WRITE_PROJECTS")
                .map(|v| split_list(&v))
                .unwrap_or_default(),
            global_writes: flag("POLARION_ALLOW_GLOBAL_WRITES"),
        }
    }

    /// Whether any write is enabled.
    pub fn writes_enabled(&self) -> bool {
        self.global_writes || !self.write_projects.is_empty()
    }

    /// A client for these settings; read-only unless writes are enabled.
    pub fn connect(&self) -> Result<Polarion> {
        let url = self
            .url
            .as_deref()
            .ok_or_else(|| CommandError::config("no server: set POLARION_URL or pass --url"))?;
        let auth = self.auth.clone().ok_or_else(|| {
            CommandError::config(
                "no credentials: set POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD",
            )
        })?;
        Ok(Polarion::builder(url, auth)
            .danger_accept_invalid_certs(self.insecure)
            .allow_writes(self.write_projects.iter().cloned())
            .allow_global_writes(self.global_writes)
            .build()?)
    }
}
