//! ftpsync — hash-based deploy over FTPS without SSH.

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use std::path::Path;

mod cli;
mod client;
mod config;
mod config_file;
mod error;
mod hasher;
mod ignore;
mod log;
mod state;
mod sync;
mod walker;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse via ArgMatches so `config::build` can tell an explicit flag from a
    // default (clap's ValueSource) when merging the config file.
    let matches = cli::Args::command().get_matches();
    let args = cli::Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    let config_path = args
        .config
        .clone()
        .unwrap_or_else(|| config::DEFAULT_CONFIG_FILE.to_string());
    let file = config_file::load(Path::new(&config_path), args.config.is_some())?;

    let cfg = config::Config::build(args, file, &matches)?;
    log::set_verbosity(cfg.verbosity);
    sync::run(cfg).await?;
    Ok(())
}
