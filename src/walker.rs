//! Local file discovery + filtering (exclude / include / .ftpignore).

use crate::config::Config;
use crate::error::{FtpSyncError, Result};
use crate::ignore::IgnoreRules;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A discovered local file: absolute path plus POSIX-relative path (vs local_dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub abs_path: PathBuf,
    pub rel_path: String,
}

/// Build a GlobSet from a list of patterns.
fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob =
            Glob::new(p).map_err(|e| FtpSyncError::Config(format!("invalid glob '{p}': {e}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| FtpSyncError::Config(format!("glob set build failed: {e}")))
}

/// Convert a path to a POSIX-style relative string (forward slashes).
fn to_posix(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// Walk the local directory and apply exclude / include / .ftpignore filters.
pub fn discover(cfg: &Config) -> Result<Vec<LocalFile>> {
    let include = build_globset(&cfg.include)?;
    let exclude = build_globset(&cfg.exclude)?;
    let whitelist_mode = !cfg.include.is_empty();

    let ignore_rules: Option<IgnoreRules> = if cfg.no_ignore_file {
        None
    } else {
        let ignore_path = cfg.local_dir.join(&cfg.ignore_file);
        Some(IgnoreRules::from_file(&cfg.local_dir, &ignore_path)?)
    };

    let mut out = Vec::new();
    for entry in WalkDir::new(&cfg.local_dir).follow_links(false) {
        let entry = entry.map_err(|e| FtpSyncError::Io(std::io::Error::other(e.to_string())))?;

        let abs_path = entry.path();
        let rel = match abs_path.strip_prefix(&cfg.local_dir) {
            Ok(r) if !r.as_os_str().is_empty() => r.to_path_buf(),
            _ => continue, // the root itself
        };
        let is_dir = entry.file_type().is_dir();

        // Honor .ftpignore for both dirs and files.
        if let Some(rules) = &ignore_rules {
            if rules.is_ignored(&rel, is_dir) {
                continue;
            }
        }

        if is_dir {
            continue;
        }
        if !entry.file_type().is_file() {
            continue; // skip symlinks / special files
        }

        let rel_posix = to_posix(&rel);

        // Never sync the state file itself if it lives in the tree.
        if rel_posix == cfg.state_file {
            continue;
        }

        if exclude.is_match(&rel_posix) {
            continue;
        }
        if whitelist_mode && !include.is_match(&rel_posix) {
            continue;
        }

        out.push(LocalFile {
            abs_path: abs_path.to_path_buf(),
            rel_path: rel_posix,
        });
    }

    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_conversion() {
        let p = Path::new("a").join("b").join("c.txt");
        assert_eq!(to_posix(&p), "a/b/c.txt");
    }

    #[test]
    fn globset_excludes() {
        let gs = build_globset(&["wp-admin/**".to_string(), "*.log".to_string()]).unwrap();
        assert!(gs.is_match("wp-admin/index.php"));
        assert!(gs.is_match("foo.log"));
        assert!(!gs.is_match("index.php"));
    }
}
