use reqwest::StatusCode;

/// Result alias used throughout this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by the Polarion SOAP client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server answered with a SOAP fault (e.g. failed login, missing permission).
    #[error("SOAP fault {code}: {message}")]
    Fault {
        code: String,
        message: String,
        /// Raw XML of the `<detail>` element, if any.
        detail: Option<String>,
    },

    /// The client is read-only and the operation may modify data; nothing was sent.
    #[error("refusing {operation}: client is read-only")]
    ReadOnly { operation: String },

    /// Logging in did not produce a session.
    #[error("login failed: {0}")]
    Login(String),

    /// The server answered with an unexpected HTTP status and no SOAP fault.
    #[error("unexpected HTTP status {status}: {}", truncate(.body))]
    Status { status: StatusCode, body: String },

    /// Transport-level failure (connection, TLS, timeout, ...).
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The response is not well-formed XML.
    #[error("malformed XML response: {0}")]
    Xml(#[from] roxmltree::Error),

    /// The response is XML but does not match the expected structure.
    #[error("failed to decode <{element}>: {message}")]
    Decode { element: String, message: String },

    /// The configured base URL is invalid.
    #[error("invalid base URL: {0}")]
    Url(#[from] url::ParseError),

    /// The client configuration is invalid.
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    pub(crate) fn decode(element: &str, message: impl Into<String>) -> Self {
        Error::Decode {
            element: element.to_owned(),
            message: message.into(),
        }
    }
}

fn truncate(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() > 200 {
        format!("{}...", body.chars().take(200).collect::<String>())
    } else {
        body.to_owned()
    }
}
