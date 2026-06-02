//! ftpsync — hash-based deploy over FTPS without SSH.

use anyhow::Result;
use clap::Parser;

mod cli;
mod client;
mod config;
mod error;
mod hasher;
mod ignore;
mod state;
mod sync;
mod walker;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    let cfg = config::Config::from_args(args)?;
    sync::run(cfg).await?;
    Ok(())
}
