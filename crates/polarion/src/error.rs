/// Errors returned by [`crate::Polarion`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server or the transport reported an error (faults, HTTP, decoding, read-only refusals).
    #[error(transparent)]
    Soap(#[from] polarion_soap::Error),

    /// The requested object does not exist or the user may not see it.
    #[error("{kind} `{id}` not found")]
    NotFound { kind: &'static str, id: String },

    /// A write was refused before anything was sent:
    /// - writes are not enabled for the project (see [`crate::PolarionBuilder::allow_writes`]),
    ///   or the write is global (users, global folders, ...) or its project could not be
    ///   determined, and global writes are not enabled
    ///   ([`crate::PolarionBuilder::allow_global_writes`]);
    /// - or the client is restricted to write targets
    ///   ([`crate::PolarionBuilder::restrict_writes`], [`crate::PolarionBuilder::allow_creates`])
    ///   and the write goes to an object that is not a target (`target`), or is never allowed in
    ///   that mode, or has no create left in the budget (`reason`).
    #[error("{operation} refused: {}", refusal(project.as_deref(), target.as_deref(), *reason))]
    WriteNotAllowed {
        operation: &'static str,
        project: Option<String>,
        /// The object that is not a write target (restricted mode).
        target: Option<String>,
        /// Why a write-restricted client refuses the write, when it is not (only) a missing target.
        reason: Option<&'static str>,
    },

    /// The commit of a write of several requests ([`crate::Polarion`] runs them as one
    /// transaction in a session of its own) was sent, but no answer came back (HTTP or status
    /// error): the changes may or may not have been committed. Check before repeating the write.
    #[error("the commit was sent without an answer; the changes may or may not be committed: {0}")]
    CommitUncertain(#[source] polarion_soap::Error),

    /// Invalid input, e.g. a value that does not fit a field's type, or no workflow action
    /// leading to the requested status.
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

fn refusal(project: Option<&str>, target: Option<&str>, reason: Option<&str>) -> String {
    match (reason, target, project) {
        (Some(reason), _, Some(project)) => format!("{reason} (project `{project}`)"),
        (Some(reason), _, None) => reason.to_owned(),
        (None, Some(target), _) => {
            format!("`{target}` is not a write target of this client")
        }
        (None, None, Some(project)) => format!("writes to project `{project}` are not allowed"),
        (None, None, None) => {
            "the write is not tied to a known project and global writes are not allowed".to_owned()
        }
    }
}
