//! FTPS client wrapper around suppaftp (async, rustls).

use crate::cli::SecureMode;
use crate::config::Config;
use crate::error::{is_not_found, FtpSyncError, Result};
use futures::io::Cursor;
use std::collections::HashSet;
use std::sync::Arc;

use futures_rustls::rustls::{ClientConfig, RootCertStore};
use futures_rustls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{AsyncRustlsConnector, AsyncRustlsFtpStream, ImplAsyncFtpStream};

/// Wrapper around an async FTP(S) stream with the helpers ftpsync needs.
pub struct Client {
    inner: AsyncRustlsFtpStream,
    cfg: Config,
    /// Remote dirs already created on this connection (skip redundant MKD/CHMOD).
    created_dirs: HashSet<String>,
}

impl Client {
    /// Connect, negotiate TLS per the secure mode, and log in, bounded by
    /// `cfg.timeout` seconds for the whole handshake.
    pub async fn connect_and_login(cfg: &Config) -> Result<Self> {
        let stream = Self::connect_stream(cfg).await?;
        Ok(Self {
            inner: stream,
            cfg: cfg.clone(),
            created_dirs: HashSet::new(),
        })
    }

    /// Tear down the current connection and establish a fresh one.
    ///
    /// We deliberately do *not* send QUIT first: after an interrupted/short
    /// transfer the server may still be waiting on the data channel, and QUIT
    /// can then block indefinitely. Reassigning `inner` drops the old stream,
    /// which just closes the socket.
    async fn reconnect(&mut self) -> Result<()> {
        self.inner = Self::connect_stream(&self.cfg).await?;
        Ok(())
    }

    async fn connect_stream(cfg: &Config) -> Result<AsyncRustlsFtpStream> {
        let dur = std::time::Duration::from_secs(cfg.timeout);
        match tokio::time::timeout(dur, Self::connect_inner(cfg)).await {
            Ok(res) => res,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "connection to {}:{} timed out after {}s",
                    cfg.server, cfg.port, cfg.timeout
                ),
            )
            .into()),
        }
    }

    async fn connect_inner(cfg: &Config) -> Result<AsyncRustlsFtpStream> {
        let addr = format!("{}:{}", cfg.server, cfg.port);

        let mut stream: AsyncRustlsFtpStream = match cfg.secure {
            SecureMode::Implicit => {
                let connector = tls_connector(cfg.insecure_tls);
                ImplAsyncFtpStream::connect_secure_implicit(addr.as_str(), connector, &cfg.server)
                    .await?
            }
            _ => ImplAsyncFtpStream::connect(addr.as_str()).await?,
        };

        if let SecureMode::Explicit = cfg.secure {
            let connector = tls_connector(cfg.insecure_tls);
            stream = stream.into_secure(connector, &cfg.server).await?;
        }

        stream.login(&cfg.username, &cfg.password).await?;
        stream.transfer_type(FileType::Binary).await?;

        if cfg.passive {
            stream.set_mode(suppaftp::Mode::Passive);
            // Ignore the IP the server advertises in its PASV reply and connect
            // the data channel to the control host instead. Misconfigured or
            // NATed servers (e.g. advertising 0.0.0.0 or a private address)
            // otherwise yield failed/empty data transfers.
            stream.set_passive_nat_workaround(true);
        } else {
            stream.set_mode(suppaftp::Mode::Active);
        }

        Ok(stream)
    }

    /// Download a remote file fully into memory. Maps 550 to `NotFound`.
    ///
    /// Some servers race the data-channel close against the "226 Complete"
    /// reply, leaving the transfer short/empty while still reporting success.
    /// Guard against it: learn the expected size via SIZE, then retry (with
    /// backoff + reconnect) until the byte count matches. A genuinely empty
    /// file matches on size 0.
    pub async fn download(&mut self, path: &str) -> Result<Vec<u8>> {
        let expected = match self.inner.size(path).await {
            Ok(n) => n,
            Err(e) if is_not_found(&e) => return Err(FtpSyncError::NotFound(path.to_string())),
            Err(e) => return Err(e.into()),
        };

        let mut last = 0usize;
        for attempt in 0..6 {
            if attempt > 0 {
                let backoff = std::time::Duration::from_millis(150 << (attempt - 1));
                tokio::time::sleep(backoff).await;
                self.reconnect().await?;
            }
            let buf = self.download_once(path).await?;
            if buf.len() == expected {
                return Ok(buf);
            }
            last = buf.len();
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("download of {path} incomplete: got {last} of {expected} bytes"),
        )
        .into())
    }

    async fn download_once(&mut self, path: &str) -> Result<Vec<u8>> {
        use futures::AsyncReadExt;
        let mut stream = match self.inner.retr_as_stream(path).await {
            Ok(s) => s,
            Err(e) if is_not_found(&e) => return Err(FtpSyncError::NotFound(path.to_string())),
            Err(e) => return Err(e.into()),
        };
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        self.inner.finalize_retr_stream(stream).await?;
        Ok(buf)
    }

    /// Upload bytes to a remote path, creating parent directories as needed.
    pub async fn upload(&mut self, path: &str, data: &[u8]) -> Result<()> {
        self.ensure_parent_dirs(path).await?;
        let mut reader = Cursor::new(data);
        self.inner.put_file(path, &mut reader).await?;
        self.apply_file_perms(path).await;
        Ok(())
    }

    /// Atomic upload: write to `{path}.ftpsync-tmp`, then rename onto `path`.
    pub async fn upload_atomic(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let tmp = format!("{path}.ftpsync-tmp");
        self.ensure_parent_dirs(path).await?;
        {
            let mut reader = Cursor::new(data);
            self.inner.put_file(&tmp, &mut reader).await?;
        }
        // Remove any pre-existing target, otherwise some servers refuse RNTO.
        let _ = self.inner.rm(path).await;
        self.inner.rename(tmp.as_str(), path).await?;
        self.apply_file_perms(path).await;
        Ok(())
    }

    /// Best-effort `SITE CHMOD` for a file, if `--file-perms` was set.
    async fn apply_file_perms(&mut self, path: &str) {
        if let Some(mode) = self.cfg.file_perms {
            self.chmod(path, mode).await;
        }
    }

    /// Best-effort `SITE CHMOD <octal> <path>`. Failures are ignored, since many
    /// servers don't support SITE CHMOD and a permission tweak shouldn't abort a deploy.
    async fn chmod(&mut self, path: &str, mode: u32) {
        let _ = self.inner.site(format!("CHMOD {mode:o} {path}")).await;
    }

    /// Returns true if a remote file exists (uses SIZE).
    pub async fn exists(&mut self, path: &str) -> Result<bool> {
        match self.inner.size(path).await {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Recursively delete the *contents* of `dir`, leaving `dir` itself in place.
    pub fn purge<'a>(
        &'a mut self,
        dir: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let entries = match self.inner.list(Some(dir)).await {
                Ok(e) => e,
                Err(e) if is_not_found(&e) => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            for line in entries {
                if let Some((name, is_dir)) = parse_list_line(&line) {
                    if name == "." || name == ".." {
                        continue;
                    }
                    let child = format!("{dir}/{name}");
                    if is_dir {
                        self.purge(&child).await?;
                        let _ = self.inner.rmdir(&child).await;
                    } else {
                        let _ = self.inner.rm(&child).await;
                    }
                }
            }
            Ok(())
        })
    }

    /// Delete a remote file (550 not-found is treated as success).
    pub async fn delete(&mut self, path: &str) -> Result<()> {
        match self.inner.rm(path).await {
            Ok(_) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Recursively list regular files under `dir`, returning POSIX paths
    /// relative to `dir`.
    pub async fn list_recursive(&mut self, dir: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let base = dir.trim_end_matches('/');
        self.list_recursive_inner(base, "", &mut out).await?;
        Ok(out)
    }

    fn list_recursive_inner<'a>(
        &'a mut self,
        abs_dir: &'a str,
        rel_prefix: &'a str,
        out: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let target = if abs_dir.is_empty() { "/" } else { abs_dir };
            let entries = self.inner.list(Some(target)).await?;
            for line in entries {
                if let Some((name, is_dir)) = parse_list_line(&line) {
                    if name == "." || name == ".." {
                        continue;
                    }
                    let rel = if rel_prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{rel_prefix}/{name}")
                    };
                    let abs = format!("{target}/{name}");
                    if is_dir {
                        self.list_recursive_inner(&abs, &rel, out).await?;
                    } else {
                        out.push(rel);
                    }
                }
            }
            Ok(())
        })
    }

    /// Ensure all parent directories of `path` exist, creating them as needed.
    /// Dirs created this connection are cached to skip redundant MKD/CHMOD.
    async fn ensure_parent_dirs(&mut self, path: &str) -> Result<()> {
        let trimmed = path.trim_start_matches('/');
        let mut components: Vec<&str> = trimmed.split('/').collect();
        components.pop(); // drop the filename
        let mut current = String::new();
        for comp in components {
            if comp.is_empty() {
                continue;
            }
            current.push('/');
            current.push_str(comp);
            if self.created_dirs.contains(&current) {
                continue;
            }
            // mkdir may fail because the dir exists; ignore those errors.
            let _ = self.inner.mkdir(&current).await;
            if let Some(mode) = self.cfg.dir_perms {
                self.chmod(&current, mode).await;
            }
            self.created_dirs.insert(current.clone());
        }
        Ok(())
    }

    /// Cleanly close the connection.
    pub async fn quit(mut self) -> Result<()> {
        self.inner.quit().await?;
        Ok(())
    }
}

/// Build a rustls connector. With `insecure`, certificate verification is disabled.
fn tls_connector(insecure: bool) -> AsyncRustlsConnector {
    let config = if insecure {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(danger::NoCertVerifier::new()))
            .with_no_client_auth()
    } else {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    };
    AsyncRustlsConnector::from(TlsConnector::from(Arc::new(config)))
}

/// Parse one line of a UNIX-style `LIST` response into (name, is_dir).
/// Returns None for lines we can't confidently parse.
fn parse_list_line(line: &str) -> Option<(String, bool)> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }
    let first = line.chars().next()?;
    // UNIX-style listing: perms links owner group size month day time name.
    // The date can contain padded/repeated spaces ("Jun  2"), so skip exactly
    // 8 whitespace-delimited fields and treat the remainder as the name (which
    // may itself contain spaces).
    if !matches!(first, 'd' | '-' | 'l' | 'p' | 'c' | 'b' | 's') {
        return None;
    }
    let mut rest = line;
    for _ in 0..8 {
        rest = rest.trim_start();
        let idx = rest.find(char::is_whitespace)?;
        rest = &rest[idx..];
    }
    let mut name = rest.trim_start().to_string();
    if name.is_empty() {
        return None;
    }
    // Strip symlink target ("name -> target").
    if first == 'l' {
        if let Some(idx) = name.find(" -> ") {
            name.truncate(idx);
        }
    }
    // Reject names a server should never send for a single entry: a path
    // separator or control characters. Either would let a hostile/buggy server
    // escape the target directory or inject FTP commands on a later operation.
    if name.contains('/') || name.chars().any(|c| c.is_control()) {
        return None;
    }
    Some((name, first == 'd'))
}

mod danger {
    use futures_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use futures_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use futures_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme};

    /// A certificate verifier that accepts everything (for self-signed certs).
    #[derive(Debug)]
    pub struct NoCertVerifier;

    impl NoCertVerifier {
        pub fn new() -> Self {
            NoCertVerifier
        }
    }

    impl ServerCertVerifier for NoCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ED25519,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unix_dir() {
        let line = "drwxr-xr-x 2 user group 4096 Jun  2 15:00 wp-content";
        assert_eq!(
            parse_list_line(line),
            Some(("wp-content".to_string(), true))
        );
    }

    #[test]
    fn parse_unix_file() {
        let line = "-rw-r--r-- 1 user group 1234 Jun  2 15:00 index.html";
        assert_eq!(
            parse_list_line(line),
            Some(("index.html".to_string(), false))
        );
    }

    #[test]
    fn parse_symlink() {
        let line = "lrwxrwxrwx 1 user group 7 Jun  2 15:00 link -> target";
        assert_eq!(parse_list_line(line), Some(("link".to_string(), false)));
    }

    #[test]
    fn rejects_path_separator_in_name() {
        let line = "-rw-r--r-- 1 user group 1 Jun  2 15:00 ../escape";
        assert_eq!(parse_list_line(line), None);
    }
}
