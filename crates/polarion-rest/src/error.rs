use reqwest::StatusCode;

use crate::jsonapi::ErrorObject;

/// Result alias used throughout this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by the Polarion REST client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server answered with a non-success status code.
    ///
    /// `errors` holds the JSON:API error objects if the body could be parsed,
    /// otherwise `body` contains the raw response text.
    #[error("Polarion API error {status}: {}", summarize(.errors, .body))]
    Api {
        status: StatusCode,
        errors: Vec<ErrorObject>,
        body: String,
    },

    /// Transport-level failure (connection, TLS, timeout, ...).
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The response body could not be decoded into the expected type.
    #[error("failed to decode response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
        body: String,
    },

    /// A request body could not be serialized.
    #[error("failed to encode request: {0}")]
    Encode(#[source] serde_json::Error),

    /// The configured base URL is invalid.
    #[error("invalid base URL: {0}")]
    Url(#[from] url::ParseError),

    /// The client configuration is invalid (e.g. a token with illegal header characters).
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    /// HTTP status code, if this is an API error.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::Http(e) => e.status(),
            _ => None,
        }
    }

    /// `true` if the server answered with 404 Not Found.
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(StatusCode::NOT_FOUND)
    }
}

fn summarize(errors: &[ErrorObject], body: &str) -> String {
    if errors.is_empty() {
        let body = body.trim();
        return if body.chars().count() > 200 {
            format!("{}...", body.chars().take(200).collect::<String>())
        } else if body.is_empty() {
            "<empty body>".to_owned()
        } else {
            body.to_owned()
        };
    }
    errors
        .iter()
        .map(|e| match (&e.title, &e.detail) {
            (Some(t), Some(d)) => format!("{t}: {d}"),
            (Some(t), None) => t.clone(),
            (None, Some(d)) => d.clone(),
            (None, None) => "<no details>".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}
