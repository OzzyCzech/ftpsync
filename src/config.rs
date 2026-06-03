//! Validated configuration derived from CLI arguments.

use crate::cli::{Args, SecureMode};
use crate::error::{FtpSyncError, Result};
use std::path::PathBuf;

/// Verbosity level derived from -v / -q.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

/// Fully validated runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub server: String,
    pub username: String,
    pub password: String,

    pub port: u16,
    pub secure: SecureMode,
    pub insecure_tls: bool,
    pub passive: bool,
    pub timeout: u64,

    pub local_dir: PathBuf,
    /// Remote dir, normalized to a POSIX path without a trailing slash (root = "").
    pub server_dir: String,
    pub state_file: String,

    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub ignore_file: String,
    pub no_ignore_file: bool,

    pub auto_init: bool,
    pub no_delete: bool,
    pub purge: Vec<String>,
    pub file_perms: Option<u32>,
    pub dir_perms: Option<u32>,
    pub concurrency: usize,
    pub dry_run: bool,
    pub verbosity: Verbosity,
}

impl Config {
    /// Build a validated configuration from parsed CLI args.
    pub fn from_args(args: Args) -> Result<Self> {
        if args.server.trim().is_empty() {
            return Err(FtpSyncError::Config("server must not be empty".into()));
        }
        if args.username.trim().is_empty() {
            return Err(FtpSyncError::Config("username must not be empty".into()));
        }
        if args.concurrency == 0 {
            return Err(FtpSyncError::Config("concurrency must be >= 1".into()));
        }
        if args.verbose && args.quiet {
            return Err(FtpSyncError::Config(
                "--verbose and --quiet are mutually exclusive".into(),
            ));
        }

        let local_dir = PathBuf::from(&args.local_dir);
        if !local_dir.is_dir() {
            return Err(FtpSyncError::Config(format!(
                "local dir does not exist or is not a directory: {}",
                local_dir.display()
            )));
        }

        // --no-auto-init overrides --auto-init (which defaults to true).
        let auto_init = !args.no_auto_init;

        let file_perms = parse_octal(args.file_perms.as_deref(), "--file-perms")?;
        let dir_perms = parse_octal(args.dir_perms.as_deref(), "--dir-perms")?;

        let verbosity = if args.quiet {
            Verbosity::Quiet
        } else if args.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
        };

        Ok(Config {
            server: args.server,
            username: args.username,
            password: args.password,
            port: args.port,
            secure: args.secure,
            insecure_tls: args.insecure_tls,
            passive: args.passive,
            timeout: args.timeout,
            local_dir,
            server_dir: normalize_remote_dir(&args.server_dir),
            state_file: args.state_file,
            include: args.include,
            exclude: args.exclude,
            ignore_file: args.ignore_file,
            no_ignore_file: args.no_ignore_file,
            auto_init,
            no_delete: args.no_delete,
            purge: args.purge,
            file_perms,
            dir_perms,
            concurrency: args.concurrency,
            dry_run: args.dry_run,
            verbosity,
        })
    }

    /// Absolute remote path for a path relative to `server_dir`.
    pub fn remote_path(&self, rel: &str) -> String {
        join_remote(&self.server_dir, rel)
    }
}

/// Parse an optional octal permission string like "0644" or "755" into its value.
fn parse_octal(value: Option<&str>, flag: &str) -> Result<Option<u32>> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let digits = raw.trim();
    if digits.is_empty() || !digits.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return Err(FtpSyncError::Config(format!(
            "{flag} must be an octal mode like 0644, got \"{raw}\""
        )));
    }
    u32::from_str_radix(digits, 8)
        .map(Some)
        .map_err(|e| FtpSyncError::Config(format!("{flag} invalid: {e}")))
}

/// Normalize a remote directory: strip trailing/duplicate slashes; root becomes "".
pub fn normalize_remote_dir(dir: &str) -> String {
    let trimmed = dir.trim();
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    parts.join("/")
}

/// Returns true if a path contains a control character (e.g. CR/LF).
///
/// Such characters in a path would be interpolated into FTP control-channel
/// commands (RETR/STOR/MKD/SITE …), which are CRLF-terminated, allowing a
/// crafted file name to inject a second command. We reject them outright.
pub fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// Join a normalized remote dir with a relative POSIX path.
pub fn join_remote(dir: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if dir.is_empty() {
        format!("/{rel}")
    } else {
        format!("/{dir}/{rel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_root() {
        assert_eq!(normalize_remote_dir("/"), "");
        assert_eq!(normalize_remote_dir(""), "");
        assert_eq!(normalize_remote_dir("/www/"), "www");
        assert_eq!(normalize_remote_dir("www//sub/"), "www/sub");
    }

    #[test]
    fn octal_parsing() {
        assert_eq!(parse_octal(None, "--x").unwrap(), None);
        assert_eq!(parse_octal(Some("0644"), "--x").unwrap(), Some(0o644));
        assert_eq!(parse_octal(Some("755"), "--x").unwrap(), Some(0o755));
        assert!(parse_octal(Some("0888"), "--x").is_err()); // 8 is not octal
        assert!(parse_octal(Some("0o600"), "--x").is_err()); // not plain octal digits
        assert!(parse_octal(Some("abc"), "--x").is_err());
    }

    #[test]
    fn control_chars() {
        assert!(has_control_chars("foo\nDELE bar"));
        assert!(has_control_chars("foo\r\nbar"));
        assert!(has_control_chars("a\tb"));
        assert!(!has_control_chars("a/b/c.txt"));
    }

    #[test]
    fn join() {
        assert_eq!(join_remote("", "a/b.txt"), "/a/b.txt");
        assert_eq!(join_remote("www", "a/b.txt"), "/www/a/b.txt");
        assert_eq!(join_remote("www", "/a/b.txt"), "/www/a/b.txt");
    }
}
