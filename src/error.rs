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
    Plugin(String, String),

    #[error("session `{0}` already exists")]
    DuplicateSession(String),

    #[error("invalid fork boundary {boundary} for session `{session_id}`: {reason}")]
    InvalidBoundary {
        session_id: String,
        boundary: u64,
        reason: String,
    },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
