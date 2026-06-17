//! Optional `.ftpsync.json` config file: pre-fills non-secret deploy options.
//!
//! Keys map 1:1 to the CLI flags (kebab-case). Two hard rules: there is no
//! `password` key (it stays in `FTPSYNC_PASSWORD` / `-p` so it never lands in
//! git), and a CLI flag always overrides the file value (see `config::build`).
//! `deny_unknown_fields` turns a mistyped key into an error rather than a
//! silently ignored setting.

use crate::cli::SecureMode;
use crate::error::{FtpSyncError, Result};
use serde::Deserialize;
use std::path::Path;

/// Deserialized `.ftpsync.json`. Every field is optional; scalars use `Option`
/// (absent = "not set, fall back to default/CLI") and list flags use `Vec`
/// (absent = empty, merged with the CLI values).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FileConfig {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub secure: Option<SecureMode>,
    pub insecure_tls: Option<bool>,
    pub passive: Option<bool>,
    pub timeout: Option<u64>,
    pub local_dir: Option<String>,
    pub server_dir: Option<String>,
    pub state_file: Option<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub ignore_file: Option<String>,
    pub no_ignore_file: Option<bool>,
    pub no_auto_init: Option<bool>,
    pub no_delete: Option<bool>,
    #[serde(default)]
    pub purge: Vec<String>,
    pub file_perms: Option<String>,
    pub dir_perms: Option<String>,
    pub concurrency: Option<usize>,
}

/// Load a config file. When `explicit` is false (the path is the default,
/// `--config` was not passed), a missing file is not an error — an empty
/// `FileConfig` is returned. Parse errors and other I/O errors always surface.
pub fn load(path: &Path, explicit: bool) -> Result<FileConfig> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| FtpSyncError::Config(format!("failed to parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
            Ok(FileConfig::default())
        }
        Err(e) => Err(FtpSyncError::Config(format!(
            "failed to read {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<FileConfig> {
        serde_json::from_str(json).map_err(|e| FtpSyncError::Config(format!("parse: {e}")))
    }

    #[test]
    fn parses_known_fields() {
        let fc = parse(
            r#"{ "server": "ftp.example.com", "secure": "explicit",
                 "exclude": ["vendor/**"], "file-perms": "0644" }"#,
        )
        .unwrap();
        assert_eq!(fc.server.as_deref(), Some("ftp.example.com"));
        assert_eq!(fc.secure, Some(SecureMode::Explicit));
        assert_eq!(fc.exclude, vec!["vendor/**".to_string()]);
        assert_eq!(fc.file_perms.as_deref(), Some("0644"));
    }

    #[test]
    fn rejects_unknown_key() {
        // A mistyped key must be an error, not silently ignored.
        assert!(parse(r#"{ "serverr": "typo" }"#).is_err());
    }

    #[test]
    fn rejects_password_key() {
        // There is deliberately no `password` field, so it is an unknown key.
        assert!(parse(r#"{ "password": "secret" }"#).is_err());
    }

    #[test]
    fn empty_object_is_all_none() {
        let fc = parse("{}").unwrap();
        assert!(fc.server.is_none());
        assert!(fc.exclude.is_empty());
    }
}
