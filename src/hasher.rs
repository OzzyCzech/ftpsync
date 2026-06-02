//! Streaming SHA-256 hashing with constant memory usage.

use crate::error::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

const CHUNK_SIZE: usize = 64 * 1024;

/// Hash a file from disk in 64 KB chunks, returning a `sha256:<hex>` string.
pub fn hash_file(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format_digest(hasher.finalize().as_slice()))
}

/// Hash an in-memory byte slice (used for auto-init of remote files).
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format_digest(hasher.finalize().as_slice())
}

fn format_digest(digest: &[u8]) -> String {
    let mut s = String::with_capacity(7 + digest.len() * 2);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_bytes() {
        // Known SHA-256 of the empty input.
        assert_eq!(
            hash_bytes(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn known_vector() {
        assert_eq!(
            hash_bytes(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn file_matches_bytes() {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("ftpsync_hash_test_{}.bin", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp).unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let file_hash = hash_file(&tmp).unwrap();
        let bytes_hash = hash_bytes(b"hello world");
        std::fs::remove_file(&tmp).ok();
        assert_eq!(file_hash, bytes_hash);
    }
}
