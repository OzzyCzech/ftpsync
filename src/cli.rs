//! CLI argument parsing via clap.

use clap::{Parser, ValueEnum};

/// Secure connection mode for the FTP control/data channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SecureMode {
    /// Plain FTP, no TLS.
    None,
    /// Explicit FTPS (AUTH TLS) — the default.
    Explicit,
    /// Implicit FTPS (TLS from the start, usually port 990).
    Implicit,
}

/// ftpsync — hash-based deploy over FTPS without SSH.
#[derive(Debug, Parser)]
#[command(
    name = "ftpsync",
    version,
    about = "Hash-based deploy over FTPS without SSH"
)]
pub struct Args {
    // --- Required ---
    /// FTP server hostname.
    #[arg(short = 's', long)]
    pub server: String,

    /// FTP username.
    #[arg(short = 'u', long)]
    pub username: String,

    /// FTP password (prefer the FTPSYNC_PASSWORD env var for CI).
    #[arg(short = 'p', long, env = "FTPSYNC_PASSWORD", hide_env_values = true)]
    pub password: String,

    // --- Connection ---
    /// FTP port.
    #[arg(long, default_value_t = 21)]
    pub port: u16,

    /// Secure mode: none|explicit|implicit.
    #[arg(long, value_enum, default_value_t = SecureMode::Explicit)]
    pub secure: SecureMode,

    /// Skip TLS certificate validation (self-signed certs).
    #[arg(long, default_value_t = false)]
    pub insecure_tls: bool,

    /// Use passive mode.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub passive: bool,

    /// Connection timeout in seconds.
    #[arg(long, default_value_t = 30)]
    pub timeout: u64,

    // --- Paths ---
    /// Local source directory.
    #[arg(short = 'l', long, default_value = ".")]
    pub local_dir: String,

    /// Remote target directory.
    #[arg(short = 'r', long, default_value = "/")]
    pub server_dir: String,

    /// State file name on the server.
    #[arg(long, default_value = ".ftpsync-state.json")]
    pub state_file: String,

    // --- Filters ---
    /// Glob to include (repeatable, enables whitelist mode).
    #[arg(long)]
    pub include: Vec<String>,

    /// Glob to exclude (repeatable).
    #[arg(long)]
    pub exclude: Vec<String>,

    /// Path to .ftpignore file.
    #[arg(long, default_value = ".ftpignore")]
    pub ignore_file: String,

    /// Don't read the .ftpignore file.
    #[arg(long, default_value_t = false)]
    pub no_ignore_file: bool,

    // --- Behavior ---
    /// Treat the server as empty on first run (upload everything) instead of
    /// hashing every remote file to bootstrap state (the default).
    #[arg(long, default_value_t = false)]
    pub no_auto_init: bool,

    /// Don't delete remote files that are missing locally.
    #[arg(long, default_value_t = false)]
    pub no_delete: bool,

    /// Remote directory to empty after deploying (repeatable), e.g. a cache dir.
    /// The directory itself is kept; only its contents are removed.
    #[arg(long)]
    pub purge: Vec<String>,

    /// chmod uploaded files to this octal mode, e.g. 0644 (best-effort, via SITE CHMOD).
    #[arg(long, value_name = "OCTAL")]
    pub file_perms: Option<String>,

    /// chmod created directories to this octal mode, e.g. 0755 (best-effort, via SITE CHMOD).
    #[arg(long, value_name = "OCTAL")]
    pub dir_perms: Option<String>,

    /// Parallel uploads.
    #[arg(short = 'j', long, default_value_t = 4)]
    pub concurrency: usize,

    /// Print actions without executing them.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// More output.
    #[arg(short = 'v', long, default_value_t = false)]
    pub verbose: bool,

    /// Less output.
    #[arg(short = 'q', long, default_value_t = false)]
    pub quiet: bool,
}
