//! Process-global verbosity + leveled stderr logging.
//!
//! Verbosity is set once in `main` and read by `log()`, which keeps it out of
//! every sync function signature. The level lives in an atomic so the parallel
//! upload workers can call `log()` concurrently and safely (`eprintln!` itself
//! already locks stderr).

use crate::config::Verbosity;
use std::sync::atomic::{AtomicU8, Ordering};

fn rank(v: Verbosity) -> u8 {
    match v {
        Verbosity::Quiet => 0,
        Verbosity::Normal => 1,
        Verbosity::Verbose => 2,
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(1); // Normal until set.

/// Set the global verbosity. Call once, early, from `main`.
pub fn set_verbosity(v: Verbosity) {
    CURRENT.store(rank(v), Ordering::Relaxed);
}

/// Print `msg` to stderr if the current verbosity is at least `level`.
///
/// Status/progress logs go to stderr; the dry-run plan goes to stdout via
/// `println!` in `sync`. Quiet (rank 0) suppresses every status log — errors
/// propagate via `Result` instead.
pub fn log(level: Verbosity, msg: &str) {
    if CURRENT.load(Ordering::Relaxed) >= rank(level) {
        eprintln!("{msg}");
    }
}
