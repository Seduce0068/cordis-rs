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

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
