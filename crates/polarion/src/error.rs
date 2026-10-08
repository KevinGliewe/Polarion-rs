/// Errors returned by [`crate::Polarion`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server or the transport reported an error (faults, HTTP, decoding, read-only refusals).
    #[error(transparent)]
    Soap(#[from] polarion_soap::Error),

    /// The requested object does not exist or the user may not see it.
    #[error("{kind} `{id}` not found")]
    NotFound { kind: &'static str, id: String },

    /// A write was refused before anything was sent: writes are not enabled for
    /// the project (see [`crate::PolarionBuilder::allow_writes`]), or the write is
    /// global (users, global folders, ...) or its project could not be determined,
    /// and global writes are not enabled ([`crate::PolarionBuilder::allow_global_writes`]).
    #[error("{operation} refused: {}", match project {
        Some(p) => format!("writes to project `{p}` are not allowed"),
        None => "the write is not tied to a known project and global writes are not allowed".to_owned(),
    })]
    WriteNotAllowed {
        operation: &'static str,
        project: Option<String>,
    },

    /// Invalid input, e.g. a value that does not fit a field's type, or no workflow action
    /// leading to the requested status.
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
