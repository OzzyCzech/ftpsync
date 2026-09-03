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

/// Returns true if the error looks transient, i.e. the same operation may well
/// succeed on a fresh connection.
///
/// FTP splits negative replies in two (RFC 959 §4.2): 4xx is "transient
/// negative completion" — the condition is temporary and the client may retry —
/// while 5xx is permanent and retrying only wastes time. Connection-level
/// failures count as transient too: the link dropped, a new one may hold.
///
/// Shared hosting hits this constantly. A server that answers
/// `450 Transfer aborted. Link to file server lost` part-way through a deploy
/// will usually take the very same file on the next attempt.
pub fn is_transient(err: &FtpSyncError) -> bool {
    match err {
        FtpSyncError::Ftp(e) => is_transient_ftp(e),
        FtpSyncError::Io(e) => is_transient_io(e),
        _ => false,
    }
}

fn is_transient_ftp(err: &suppaftp::FtpError) -> bool {
    match err {
        suppaftp::FtpError::ConnectionError(e) => is_transient_io(e),
        // An aborted transfer can leave the control channel mid-reply, so the
        // next response parses as garbage or the data channel still looks open.
        // Both clear on reconnect.
        suppaftp::FtpError::BadResponse | suppaftp::FtpError::DataConnectionAlreadyOpen => true,
        suppaftp::FtpError::UnexpectedResponse(resp) => (400..500).contains(&(resp.status as u32)),
        _ => false,
    }
}

fn is_transient_io(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use suppaftp::types::Response;
    use suppaftp::{FtpError, Status};

    fn response(status: Status) -> FtpSyncError {
        FtpSyncError::Ftp(FtpError::UnexpectedResponse(Response {
            status,
            body: Vec::new(),
        }))
    }

    #[test]
    fn transient_for_4xx_responses() {
        // The failure that motivated this: 450 mid-deploy on shared hosting.
        assert!(is_transient(&response(Status::RequestFileActionIgnored)));
        assert!(is_transient(&response(Status::NotAvailable)));
        assert!(is_transient(&response(Status::CannotOpenDataConnection)));
        assert!(is_transient(&response(Status::TransferAborted)));
    }

    #[test]
    fn permanent_for_5xx_responses() {
        assert!(!is_transient(&response(Status::FileUnavailable)));
        assert!(!is_transient(&response(Status::BadCommand)));
    }

    #[test]
    fn transient_for_dropped_connections() {
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::TimedOut,
        ] {
            let bare = FtpSyncError::Io(std::io::Error::from(kind));
            assert!(is_transient(&bare), "{kind:?} should be transient");
            let wrapped = FtpSyncError::Ftp(FtpError::ConnectionError(std::io::Error::from(kind)));
            assert!(is_transient(&wrapped), "{kind:?} should be transient");
        }
    }

    #[test]
    fn permanent_for_local_and_config_errors() {
        assert!(!is_transient(&FtpSyncError::Config("bad".into())));
        assert!(!is_transient(&FtpSyncError::PathTraversal("../x".into())));
        assert!(!is_transient(&FtpSyncError::UnsupportedStateVersion(9)));
        assert!(!is_transient(&FtpSyncError::Io(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        ))));
    }
}
