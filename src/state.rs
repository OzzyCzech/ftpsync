//! State file (`.ftpsync-state.json`) (de)serialization.
//!
//! The on-disk format is shared with the parallel Bun implementation:
//! `{ version: 1, tool, updated, files: { path: { hash, size, uploaded } } }`.

use crate::error::{FtpSyncError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum allowed state file size (sanity check against abuse / corruption).
pub const MAX_STATE_SIZE: usize = 100 * 1024 * 1024;

/// Current state file format version.
pub const STATE_VERSION: u32 = 1;

/// Per-file metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    /// SHA-256 hash, formatted as `sha256:<hex>`.
    pub hash: String,
    /// File size in bytes.
    pub size: u64,
    /// RFC 3339 timestamp of last upload.
    pub uploaded: String,
}

/// The full state document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub version: u32,
    pub tool: String,
    pub updated: String,
    /// Map of POSIX path (relative to server-dir) -> file entry.
    /// BTreeMap keeps output deterministic across runs and tools.
    pub files: BTreeMap<String, FileEntry>,
}

impl State {
    /// An empty state stamped with the current tool/version.
    pub fn empty() -> Self {
        State {
            version: STATE_VERSION,
            tool: tool_string(),
            updated: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            files: BTreeMap::new(),
        }
    }

    /// Parse a state document from raw JSON bytes, with size + version + path checks.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_STATE_SIZE {
            return Err(FtpSyncError::StateTooLarge {
                size: bytes.len(),
                max: MAX_STATE_SIZE,
            });
        }
        let state: State = serde_json::from_slice(bytes)?;
        if state.version != STATE_VERSION {
            return Err(FtpSyncError::UnsupportedStateVersion(state.version));
        }
        for path in state.files.keys() {
            if has_path_traversal(path) {
                return Err(FtpSyncError::PathTraversal(path.clone()));
            }
        }
        Ok(state)
    }

    /// Serialize to pretty JSON bytes, refreshing the `updated`/`tool` fields.
    pub fn render_json(&mut self) -> Result<Vec<u8>> {
        self.tool = tool_string();
        self.updated = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Insert/update an entry for `path`.
    pub fn set(&mut self, path: &str, hash: String, size: u64) {
        self.files.insert(
            path.to_string(),
            FileEntry {
                hash,
                size,
                uploaded: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            },
        );
    }

    /// Remove an entry for `path`.
    pub fn remove(&mut self, path: &str) {
        self.files.remove(path);
    }
}

/// The `tool` field value, e.g. "ftpsync 0.1.0".
pub fn tool_string() -> String {
    format!("ftpsync {}", env!("CARGO_PKG_VERSION"))
}

/// Returns true if a POSIX path contains a `..` component or is absolute.
pub fn has_path_traversal(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    path.split('/').any(|c| c == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = State::empty();
        s.set("assets/css/style.css", "sha256:abc123".to_string(), 4096);
        let bytes = s.render_json().unwrap();
        let parsed = State::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.files.len(), 1);
        let entry = parsed.files.get("assets/css/style.css").unwrap();
        assert_eq!(entry.hash, "sha256:abc123");
        assert_eq!(entry.size, 4096);
    }

    #[test]
    fn bun_compatible_shape() {
        // A document produced by the Bun side must parse here.
        let json = r#"{
            "version": 1,
            "tool": "ftpsync 0.1.0",
            "updated": "2026-06-02T15:00:00Z",
            "files": {
                "index.html": { "hash": "sha256:deadbeef", "size": 12, "uploaded": "2026-06-02T15:00:00Z" }
            }
        }"#;
        let s = State::from_bytes(json.as_bytes()).unwrap();
        assert_eq!(s.files["index.html"].size, 12);
    }

    #[test]
    fn rejects_bad_version() {
        let json = r#"{"version":2,"tool":"x","updated":"now","files":{}}"#;
        assert!(matches!(
            State::from_bytes(json.as_bytes()),
            Err(FtpSyncError::UnsupportedStateVersion(2))
        ));
    }

    #[test]
    fn rejects_path_traversal() {
        let json = r#"{"version":1,"tool":"x","updated":"now","files":{"../secret":{"hash":"sha256:a","size":1,"uploaded":"now"}}}"#;
        assert!(matches!(
            State::from_bytes(json.as_bytes()),
            Err(FtpSyncError::PathTraversal(_))
        ));
    }

    #[test]
    fn traversal_detection() {
        assert!(has_path_traversal("../etc"));
        assert!(has_path_traversal("a/../b"));
        assert!(has_path_traversal("/abs"));
        assert!(!has_path_traversal("a/b/c.txt"));
    }
}
