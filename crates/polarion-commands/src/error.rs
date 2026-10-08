use polarion::soap::Error as SoapError;

/// A failed command, with a stable code for machine consumers.
#[derive(Debug)]
pub struct CommandError {
    /// `notFound`, `readOnly`, `invalid`, `fault`, `login`, `http`, `config` or `other`.
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid",
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self {
            code: "config",
            message: message.into(),
        }
    }

    /// Process exit code for command-line use: 3 for "not found", 1 otherwise.
    pub fn exit_code(&self) -> u8 {
        if self.code == "notFound" { 3 } else { 1 }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<polarion::Error> for CommandError {
    fn from(e: polarion::Error) -> Self {
        let code = match &e {
            polarion::Error::NotFound { .. } => "notFound",
            polarion::Error::WriteNotAllowed { .. } => "readOnly",
            polarion::Error::Invalid(_) => "invalid",
            polarion::Error::Soap(s) => match s {
                SoapError::Fault { .. } => "fault",
                SoapError::Login(_) => "login",
                SoapError::ReadOnly { .. } => "readOnly",
                SoapError::Http(_) | SoapError::Status { .. } => "http",
                SoapError::Url(_) | SoapError::Config(_) => "config",
                _ => "other",
            },
        };
        Self {
            code,
            message: e.to_string(),
        }
    }
}

impl From<polarion::soap::Error> for CommandError {
    fn from(e: polarion::soap::Error) -> Self {
        polarion::Error::from(e).into()
    }
}

pub type Result<T, E = CommandError> = std::result::Result<T, E>;
