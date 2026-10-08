/// Errors returned by [`crate::Polarion`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server or the transport reported an error (faults, HTTP, decoding, read-only refusals).
    #[error(transparent)]
    Soap(#[from] polarion_soap::Error),

    /// The requested object does not exist or the user may not see it.
    #[error("{kind} `{id}` not found")]
    NotFound { kind: &'static str, id: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
