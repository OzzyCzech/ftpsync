//! .ftpignore parsing using gitignore-style semantics.

use crate::error::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// A compiled set of .ftpignore rules. Wraps the `ignore` crate's Gitignore.
pub struct IgnoreRules {
    inner: Gitignore,
}

impl IgnoreRules {
    /// Build ignore rules from a .ftpignore file located at `path`, rooted at `root`.
    /// If the file does not exist, returns an empty (matches-nothing) rule set.
    pub fn from_file(root: &Path, path: &Path) -> Result<Self> {
        let mut builder = GitignoreBuilder::new(root);
        if path.exists() {
            // add() returns Option<Error>; surface it as an io error.
            if let Some(err) = builder.add(path) {
                return Err(
                    std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()).into(),
                );
            }
        }
        let inner = builder
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Self { inner })
    }

    /// Build ignore rules directly from a list of pattern lines (used in tests).
    #[cfg(test)]
    pub fn from_lines(root: &Path, lines: &[&str]) -> Result<Self> {
        let mut builder = GitignoreBuilder::new(root);
        for line in lines {
            builder
                .add_line(None, line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        }
        let inner = builder
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Self { inner })
    }

    /// Returns true if the given relative path should be ignored, honoring negations.
    pub fn is_ignored(&self, rel_path: &Path, is_dir: bool) -> bool {
        self.inner.matched(rel_path, is_dir).is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rules(lines: &[&str]) -> IgnoreRules {
        IgnoreRules::from_lines(Path::new("/root"), lines).unwrap()
    }

    #[test]
    fn basic_patterns() {
        let r = rules(&["*.log", "node_modules/", "# comment", ".env*"]);
        assert!(r.is_ignored(&PathBuf::from("app.log"), false));
        assert!(r.is_ignored(&PathBuf::from("node_modules"), true));
        assert!(r.is_ignored(&PathBuf::from(".env.local"), false));
        assert!(!r.is_ignored(&PathBuf::from("main.rs"), false));
    }

    #[test]
    fn negation() {
        let r = rules(&["*.log", "!important.log"]);
        assert!(r.is_ignored(&PathBuf::from("debug.log"), false));
        assert!(!r.is_ignored(&PathBuf::from("important.log"), false));
    }
}
