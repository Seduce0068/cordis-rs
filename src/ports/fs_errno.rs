//! Pure Rust port of `@deepseek-ai/dsh-fs/errno`.
//!
//! Source: `packages/fs/fs/src/errno.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Classifies host errno failures into shared filesystem error codes.

use std::fmt;

/// Shared filesystem error codes (upstream `FsError.code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FsErrorCode {
    NotFound,
    PermissionDenied,
    NotRegularFile,
    IoError,
}

impl FsErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "FS_NOT_FOUND",
            Self::PermissionDenied => "FS_PERMISSION_DENIED",
            Self::NotRegularFile => "FS_NOT_REGULAR_FILE",
            Self::IoError => "FS_IO_ERROR",
        }
    }
}

/// A classified filesystem error, preserving the original error as cause.
#[derive(Debug, Clone)]
pub struct FsError {
    pub code: FsErrorCode,
    pub message: String,
    cause: std::sync::Arc<std::io::Error>,
}

impl FsError {
    /// The original host error (upstream `cause`).
    pub fn cause(&self) -> &std::io::Error {
        &self.cause
    }
}

impl PartialEq for FsError {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code && self.message == other.message
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// Classify an `std::io::Error` by its `ErrorKind` first so the mapping works
/// on every platform, then fall back to raw OS errno for kinds Rust does not
/// distinguish (swarm review R3-IV).
///
/// `cause` keeps only the original message: it is a freshly built
/// `Error::other(...)`, so `source().raw_os_error()` is `None`. Callers must
/// re-classify via this function, not via `source()`.
pub fn fs_error_from_errno(operation: &str, display_path: &str, error: &std::io::Error) -> FsError {
    use std::io::ErrorKind;

    let failure = format!("cannot {operation} \"{display_path}\"");
    let cause = std::sync::Arc::new(std::io::Error::other(error.to_string()));

    let code = match error.kind() {
        // ENOENT maps to NotFound; ENOTDIR has its own kind on recent Rust.
        ErrorKind::NotFound | ErrorKind::NotADirectory => Some(FsErrorCode::NotFound),
        ErrorKind::PermissionDenied => Some(FsErrorCode::PermissionDenied),
        // EISDIR has its own kind on recent Rust.
        ErrorKind::IsADirectory => Some(FsErrorCode::NotRegularFile),
        _ => None,
    };

    let Some(code) = code else {
        // Fallback for errnos whose kind Rust does not map distinctly.
        return match error.raw_os_error() {
            Some(2) | Some(20) => FsError {
                code: FsErrorCode::NotFound,
                message: format!("{failure}: not found"),
                cause,
            },
            Some(13) | Some(1) => FsError {
                code: FsErrorCode::PermissionDenied,
                message: format!("{failure}: permission denied"),
                cause,
            },
            Some(21) => FsError {
                code: FsErrorCode::NotRegularFile,
                message: format!("{failure}: not a regular file"),
                cause,
            },
            _ => FsError {
                code: FsErrorCode::IoError,
                message: format!("{failure}: {cause}"),
                cause,
            },
        };
    };

    let suffix = match code {
        FsErrorCode::NotFound => "not found",
        FsErrorCode::PermissionDenied => "permission denied",
        FsErrorCode::NotRegularFile => "not a regular file",
        FsErrorCode::IoError => unreachable!(),
    };
    FsError {
        code,
        message: format!("{failure}: {suffix}"),
        cause,
    }
}
