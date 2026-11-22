use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("service `{0}` is not registered")]
    ServiceNotFound(String),

    #[error("service `{0}` is registered with a different type")]
    ServiceTypeMismatch(String),

    #[error("service `{0}` is already registered")]
    DuplicateService(String),

    #[error("context is unloading")]
    Unloading,

    #[error("plugin `{0}` failed: {1}")]
    Plugin(String, #[source] Box<Error>),

    #[error("session `{0}` already exists")]
    DuplicateSession(String),

    #[error("invalid fork boundary {boundary} for session `{session_id}`: {reason}")]
    InvalidBoundary {
        session_id: String,
        boundary: u64,
        reason: String,
    },

    /// A domain error carrying a stable category tag so callers can branch
    /// without parsing the display string (swarm review R1-AP02).
    #[error("{0}: {1}")]
    Domain(&'static str, String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// Domain errors carrying a stable category tag. Existing `Other`-based
// conversions elsewhere are intentionally left untouched for backwards
// compatibility (swarm review R1-AP02).
impl From<crate::services::shell::ShellError> for Error {
    fn from(error: crate::services::shell::ShellError) -> Self {
        Self::Domain("shell", error.to_string())
    }
}

impl From<crate::services::tools::ToolError> for Error {
    fn from(error: crate::services::tools::ToolError) -> Self {
        Self::Domain("tool", error.to_string())
    }
}

impl From<crate::plugin_ports::PersistenceError> for Error {
    fn from(error: crate::plugin_ports::PersistenceError) -> Self {
        Self::Domain("persistence", error.to_string())
    }
}
