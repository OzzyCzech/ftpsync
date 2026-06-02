//! Error types for ftpsync.

use thiserror::Error;

/// Errors produced by the ftpsync core logic.
#[derive(Error, Debug)]
pub enum FtpSyncError {
    #[error("FTP error: {0}")]
    Ftp(#[from] suppaftp::FtpError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("state file too large: {size} bytes (max {max} bytes)")]
    StateTooLarge { size: usize, max: usize },

    #[error("unsupported state file version: {0}")]
    UnsupportedStateVersion(u32),

    #[error("path traversal detected in path: {0}")]
    PathTraversal(String),

    #[error("remote file not found: {0}")]
    NotFound(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, FtpSyncError>;

/// Returns true if the given FTP error represents a "not found" / 550 condition.
pub fn is_not_found(err: &suppaftp::FtpError) -> bool {
    matches!(
        err,
        suppaftp::FtpError::UnexpectedResponse(resp)
            if resp.status == suppaftp::Status::FileUnavailable
    )
}
