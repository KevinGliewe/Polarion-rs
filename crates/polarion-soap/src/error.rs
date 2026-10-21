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

    /// The client's write filter ([`crate::ClientBuilder::write_filter`]) refused the operation;
    /// nothing was sent.
    #[error("refusing {operation}: not allowed by the client's write filter")]
    WriteFiltered { operation: String },

    /// The client is [`crate::ClientBuilder::transaction_only`] and the write was not sent inside
    /// a transaction; nothing was sent.
    #[error("refusing {operation}: no transaction is open on this transaction-only client")]
    NoTransaction { operation: String },

    /// Logging in did not produce a session.
    #[error("login failed: {0}")]
    Login(String),

    /// The session expired while an explicit transaction was open. The transaction is lost; the
    /// next call logs in again. (Outside a transaction the client logs in again by itself.)
    /// A [`crate::ClientBuilder::transaction_only`] client returns it for every login after its
    /// first one, a session renewal included: its transaction cannot survive a new session.
    #[error(
        "the Polarion session expired during an explicit transaction; its uncommitted changes are lost"
    )]
    SessionExpired,

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
