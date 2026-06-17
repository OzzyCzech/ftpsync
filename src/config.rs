//! Validated configuration derived from CLI arguments.

use crate::cli::{Args, SecureMode};
use crate::config_file::FileConfig;
use crate::error::{FtpSyncError, Result};
use clap::parser::ValueSource;
use clap::ArgMatches;
use std::path::PathBuf;

/// Default config file name, looked up in the current directory.
pub const DEFAULT_CONFIG_FILE: &str = ".ftpsync.json";

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

    /// Config file path, excluded from upload the same way `state_file` is.
    pub config_file: String,
}

/// True if the user explicitly supplied the arg (on the CLI or via env),
/// as opposed to clap filling in a default or leaving it unset. This is how
/// we let a CLI flag override the config file while a default does not.
fn cli_set(matches: &ArgMatches, id: &str) -> bool {
    matches!(
        matches.value_source(id),
        Some(ValueSource::CommandLine) | Some(ValueSource::EnvVariable)
    )
}

impl Config {
    /// Build a validated configuration from CLI args, the (already loaded)
    /// config file, and clap's match metadata.
    ///
    /// Precedence is default -> file -> CLI: a scalar takes the file value
    /// unless the flag was set explicitly; list flags (include/exclude/purge)
    /// merge the file's entries with the CLI's, CLI appended last.
    pub fn build(args: Args, file: FileConfig, matches: &ArgMatches) -> Result<Self> {
        // Scalar with a clap default: file wins over the default, CLI wins over both.
        macro_rules! pick {
            ($id:literal, $cli:expr, $file:expr) => {
                if cli_set(matches, $id) {
                    $cli
                } else {
                    $file.unwrap_or($cli)
                }
            };
        }

        // server/username have no clap default, so an unset CLI value is `None`
        // and falls back to the file. The password is intentionally absent from
        // the file (it must not land in git); it comes from `-p` / the env only.
        let server = require(args.server.or(file.server), "server", "--server")?;
        let username = require(args.username.or(file.username), "username", "--username")?;
        let password = require(
            args.password,
            "password",
            "--password / the FTPSYNC_PASSWORD env var",
        )?;

        let concurrency = pick!("concurrency", args.concurrency, file.concurrency);
        if concurrency == 0 {
            return Err(FtpSyncError::Config("concurrency must be >= 1".into()));
        }
        if args.verbose && args.quiet {
            return Err(FtpSyncError::Config(
                "--verbose and --quiet are mutually exclusive".into(),
            ));
        }

        let local_dir = PathBuf::from(pick!("local_dir", args.local_dir, file.local_dir));
        if !local_dir.is_dir() {
            return Err(FtpSyncError::Config(format!(
                "local dir does not exist or is not a directory: {}",
                local_dir.display()
            )));
        }

        // --no-auto-init overrides --auto-init (which defaults to true).
        let auto_init = !pick!("no_auto_init", args.no_auto_init, file.no_auto_init);

        // These have no clap default either, so `.or` falls back to the file.
        let file_perms_raw = args.file_perms.or(file.file_perms);
        let dir_perms_raw = args.dir_perms.or(file.dir_perms);
        let file_perms = parse_octal(file_perms_raw.as_deref(), "--file-perms")?;
        let dir_perms = parse_octal(dir_perms_raw.as_deref(), "--dir-perms")?;

        // List flags merge: file entries first, CLI entries appended last.
        let include = merge_vec(file.include, args.include);
        let exclude = merge_vec(file.exclude, args.exclude);
        let purge = merge_vec(file.purge, args.purge);

        let verbosity = if args.quiet {
            Verbosity::Quiet
        } else if args.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
        };

        let config_file = args
            .config
            .unwrap_or_else(|| DEFAULT_CONFIG_FILE.to_string());

        Ok(Config {
            server,
            username,
            password,
            port: pick!("port", args.port, file.port),
            secure: pick!("secure", args.secure, file.secure),
            insecure_tls: pick!("insecure_tls", args.insecure_tls, file.insecure_tls),
            passive: pick!("passive", args.passive, file.passive),
            timeout: pick!("timeout", args.timeout, file.timeout),
            local_dir,
            server_dir: normalize_remote_dir(&pick!(
                "server_dir",
                args.server_dir,
                file.server_dir
            )),
            state_file: pick!("state_file", args.state_file, file.state_file),
            include,
            exclude,
            ignore_file: pick!("ignore_file", args.ignore_file, file.ignore_file),
            no_ignore_file: pick!("no_ignore_file", args.no_ignore_file, file.no_ignore_file),
            auto_init,
            no_delete: pick!("no_delete", args.no_delete, file.no_delete),
            purge,
            file_perms,
            dir_perms,
            concurrency,
            dry_run: args.dry_run,
            verbosity,
            config_file,
        })
    }

    /// Absolute remote path for a path relative to `server_dir`.
    pub fn remote_path(&self, rel: &str) -> String {
        join_remote(&self.server_dir, rel)
    }
}

/// Unwrap a required credential that may come from the CLI or the config file,
/// rejecting an absent or blank value with a hint at where to set it.
fn require(value: Option<String>, name: &str, where_: &str) -> Result<String> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(FtpSyncError::Config(format!(
            "{name} is required: set {where_} or put \"{name}\" in {DEFAULT_CONFIG_FILE}"
        ))),
    }
}

/// Merge a list flag: the config file's entries first, then the CLI's appended
/// last (so a CLI value wins on conflict and is applied later).
fn merge_vec(mut file: Vec<String>, cli: Vec<String>) -> Vec<String> {
    file.extend(cli);
    file
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
    use clap::{CommandFactory, FromArgMatches};

    /// Build a Config from a CLI argv (without the program name) and a file config.
    fn build_from(argv: &[&str], file: FileConfig) -> Result<Config> {
        let mut full = vec!["ftpsync"];
        full.extend_from_slice(argv);
        let matches = Args::command().get_matches_from(full);
        let args = Args::from_arg_matches(&matches).unwrap();
        Config::build(args, file, &matches)
    }

    #[test]
    fn cli_overrides_file_scalar() {
        let file = FileConfig {
            server: Some("file-host".into()),
            username: Some("file-user".into()),
            server_dir: Some("/www".into()),
            ..Default::default()
        };
        // server-dir given on the CLI must win over the file's "/www".
        let cfg = build_from(&["-p", "pw", "--server-dir", "/public"], file).unwrap();
        assert_eq!(cfg.server, "file-host"); // from file (no CLI value)
        assert_eq!(cfg.server_dir, "public"); // CLI wins, normalized
    }

    #[test]
    fn file_fills_when_cli_is_default() {
        let file = FileConfig {
            server: Some("file-host".into()),
            username: Some("file-user".into()),
            server_dir: Some("/www".into()),
            concurrency: Some(8),
            secure: Some(SecureMode::Implicit),
            ..Default::default()
        };
        let cfg = build_from(&["-p", "pw"], file).unwrap();
        assert_eq!(cfg.server_dir, "www");
        assert_eq!(cfg.concurrency, 8);
        assert_eq!(cfg.secure, SecureMode::Implicit);
    }

    #[test]
    fn vec_flags_merge_with_cli_last() {
        let file = FileConfig {
            server: Some("h".into()),
            username: Some("u".into()),
            exclude: vec!["vendor/**".into()],
            ..Default::default()
        };
        let cfg = build_from(&["-p", "pw", "--exclude", "uploads/**"], file).unwrap();
        assert_eq!(cfg.exclude, vec!["vendor/**", "uploads/**"]); // file first, CLI last
    }

    #[test]
    fn missing_required_field_errors() {
        // No server anywhere -> a clear error after the merge.
        let err = build_from(&["-u", "u", "-p", "pw"], FileConfig::default()).unwrap_err();
        assert!(matches!(err, FtpSyncError::Config(_)));
    }

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
