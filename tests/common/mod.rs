//! Shared fixtures for the Docker-backed FTP integration tests.
//!
//! One container serves the whole test binary: starting and removing a
//! container per test churns Docker's published ports, and connections then
//! land on a proxy whose server is already going away. Instead each test takes
//! a `Scope` — its own directory on the shared server, wiped on entry — and a
//! global lock keeps the tests from overlapping on it.
//!
//! The container is removed at the start of the next run rather than at the
//! end of this one (Rust runs no teardown after the last test). To clear it by
//! hand: `docker rm -f ftpsync-integration-test`.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Multi-arch vsftpd image, so this runs natively on both amd64 CI and arm64
/// laptops.
const IMAGE: &str = "garethflowers/ftp-server:latest";
const CONTAINER: &str = "ftpsync-integration-test";
const USER: &str = "tester";
const PASS: &str = "s3cret";
const CONTROL_PORT: u16 = 2121;
// Published 1:1 — the server advertises these numbers over PASV, and the range
// is baked into the image's vsftpd.conf.
const PASV_MIN: u16 = 40000;
const PASV_MAX: u16 = 40009;
/// The FTP user's (chrooted) home inside the container.
const REMOTE_ROOT: &str = "/home/tester";

fn docker(args: &[&str]) -> Output {
    Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run `docker {}`: {e}", args.join(" ")))
}

fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True when the shared container is up and serving.
fn container_running() -> bool {
    let out = docker(&["inspect", "-f", "{{.State.Running}}", CONTAINER]);
    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
}

/// Bring the shared server up if it is not already, returning `false` when
/// Docker is unusable so the tests can skip rather than fail.
fn ensure_container() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if !*AVAILABLE.get_or_init(docker_available) {
        eprintln!("SKIP: docker is not available");
        return false;
    }
    if container_running() {
        return true;
    }
    docker(&["rm", "-f", CONTAINER]);
    await_ports_released();

    let out = docker(&[
        "run",
        "--rm",
        "-d",
        "--name",
        CONTAINER,
        "-p",
        &format!("{CONTROL_PORT}:21"),
        "-p",
        &format!("{PASV_MIN}-{PASV_MAX}:{PASV_MIN}-{PASV_MAX}"),
        "-e",
        &format!("FTP_USER={USER}"),
        "-e",
        &format!("FTP_PASS={PASS}"),
        IMAGE,
    ]);
    assert!(
        out.status.success(),
        "could not start {IMAGE}:\n{}\nports {CONTROL_PORT} and \
         {PASV_MIN}-{PASV_MAX} must be free on the host",
        String::from_utf8_lossy(&out.stderr)
    );
    await_banner();
    true
}

/// Docker frees published ports asynchronously, so a leftover container from a
/// previous run can still hold them when this one starts.
fn await_ports_released() {
    let deadline = Instant::now() + Duration::from_secs(30);
    let ports: Vec<u16> = std::iter::once(CONTROL_PORT)
        .chain(PASV_MIN..=PASV_MAX)
        .collect();
    while Instant::now() < deadline {
        let busy = ports.iter().any(|&port| {
            let addr: SocketAddr = ([127, 0, 0, 1], port).into();
            TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
        });
        if !busy {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("ports {CONTROL_PORT} and {PASV_MIN}-{PASV_MAX} must be free on the host");
}

/// Wait for a `220` greeting. Polling the protocol rather than the image's
/// healthcheck keeps this independent of how the image is built.
fn await_banner() {
    let addr: SocketAddr = ([127, 0, 0, 1], CONTROL_PORT).into();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(mut sock) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            sock.set_read_timeout(Some(Duration::from_secs(1))).ok();
            let mut buf = [0u8; 3];
            if sock.read_exact(&mut buf).is_ok() && &buf == b"220" {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("FTP server did not answer on port {CONTROL_PORT} within 60s");
}

fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    // A panicking test poisons the lock, but every scope wipes its own
    // directory on entry, so there is nothing for the poison to protect.
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// One test's private directory on the shared server.
pub struct Scope {
    name: String,
    _guard: MutexGuard<'static, ()>,
}

fn scope(name: &str) -> Option<Scope> {
    let guard = test_lock();
    if !ensure_container() {
        return None;
    }
    let scope = Scope {
        name: name.to_string(),
        _guard: guard,
    };
    scope.reset();
    Some(scope)
}

/// Run a test body against its own scope on the shared server.
///
/// The image's vsftpd occasionally segfaults (exit 139) under ordinary deploy
/// traffic — recursive LIST/DELE, or several workers logging in at once. The
/// crash reproduces identically with pre-suppaftp-10 builds of ftpsync, so it
/// is a fragility of the server rather than of this client. When the container
/// dies mid-test the body is retried once on a fresh one; a failure with the
/// server still alive is a real one and propagates immediately.
pub fn with_server(name: &str, body: impl Fn(&Scope) + std::panic::RefUnwindSafe) {
    let Some(first) = scope(name) else {
        return;
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&first)));
    let Err(panic) = outcome else {
        return;
    };
    if container_running() {
        std::panic::resume_unwind(panic);
    }
    drop(first);
    eprintln!("NOTE: the FTP server died mid-test; retrying `{name}` on a fresh container");
    let Some(retry) = scope(name) else {
        return;
    };
    body(&retry);
}

impl Scope {
    /// Absolute path inside the container.
    fn abs(&self, rel: &str) -> String {
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            format!("{REMOTE_ROOT}/{}", self.name)
        } else {
            format!("{REMOTE_ROOT}/{}/{rel}", self.name)
        }
    }

    /// Start from an empty directory the FTP user owns.
    fn reset(&self) {
        let root = self.abs("");
        let out = self.sh(&format!(
            "rm -rf {root} && mkdir -p {root} && chown -R {USER}:{USER} {root}"
        ));
        assert!(
            out.status.success(),
            "could not reset scope {}:\n{}",
            self.name,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn sh(&self, script: &str) -> Output {
        docker(&["exec", CONTAINER, "sh", "-c", script])
    }

    /// Every regular file in this scope, as sorted paths relative to its root.
    pub fn files(&self) -> Vec<String> {
        let root = self.abs("");
        let out = self.sh(&format!("find {root} -type f"));
        let prefix = format!("{root}/");
        let mut files: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .map(str::to_string)
            .collect();
        files.sort();
        files
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.sh(&format!("test -e {}", self.abs(rel)))
            .status
            .success()
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.sh(&format!("test -d {}", self.abs(rel)))
            .status
            .success()
    }

    pub fn read(&self, rel: &str) -> String {
        let out = self.sh(&format!("cat {}", self.abs(rel)));
        assert!(out.status.success(), "no such remote file: {rel}");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Copy a remote file out of the container for a byte-exact comparison.
    pub fn read_bytes(&self, rel: &str, scratch: &Path) -> Vec<u8> {
        let dest = scratch.join("docker-cp-out");
        std::fs::remove_file(&dest).ok();
        let out = docker(&[
            "cp",
            &format!("{CONTAINER}:{}", self.abs(rel)),
            dest.to_str().expect("scratch path is not utf-8"),
        ]);
        assert!(
            out.status.success(),
            "docker cp of {rel} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = std::fs::read(&dest).expect("copied file is unreadable");
        std::fs::remove_file(&dest).ok();
        bytes
    }

    /// Plant remote content, so a test can start from a non-empty server.
    pub fn seed(&self, rel: &str, contents: &str) {
        let abs = self.abs(rel);
        let parent = Path::new(&abs)
            .parent()
            .expect("seeded path has no parent")
            .to_string_lossy()
            .to_string();
        assert!(self.sh(&format!("mkdir -p {parent}")).status.success());

        let mut child = Command::new("docker")
            .args(["exec", "-i", CONTAINER, "sh", "-c", &format!("cat > {abs}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn docker exec");
        child
            .stdin
            .as_mut()
            .expect("stdin was not piped")
            .write_all(contents.as_bytes())
            .expect("failed to write seeded content");
        let out = child.wait_with_output().expect("docker exec failed");
        assert!(out.status.success(), "could not seed {rel}");

        // The FTP user must be able to overwrite what we planted as root.
        let root = self.abs("");
        assert!(self
            .sh(&format!("chown -R {USER}:{USER} {root}"))
            .status
            .success());
    }

    /// An `ftpsync` invocation deploying `local_dir` into this scope.
    pub fn ftpsync(&self, local_dir: &Path) -> Command {
        self.ftpsync_at(local_dir, "")
    }

    /// Upload concurrency used by the tests.
    ///
    /// Two workers plus the control connection still cover the parallel upload
    /// path, but the image's vsftpd segfaults (exit 139) when ftpsync's default
    /// of four workers logs in at once — and takes the rest of the suite with
    /// it. That crash reproduces identically with pre-suppaftp-10 builds, so it
    /// is a fragility of this server, not of the client.
    const CONCURRENCY: &'static str = "2";

    /// As [`Scope::ftpsync`], but targeting `subdir` below the scope root.
    pub fn ftpsync_at(&self, local_dir: &Path, subdir: &str) -> Command {
        let server_dir = if subdir.is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{}/{subdir}", self.name)
        };
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ftpsync"));
        cmd.args(["--server", "127.0.0.1"])
            .args(["--port", &CONTROL_PORT.to_string()])
            .args(["--username", USER])
            .args(["--password", PASS])
            .args(["--secure", "none"])
            .args(["--concurrency", Self::CONCURRENCY])
            .args(["--server-dir", &server_dir])
            .arg("--local-dir")
            .arg(local_dir)
            .arg("--verbose");
        cmd
    }
}

/// Run `ftpsync` and return its combined output, asserting it succeeded.
pub fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("failed to execute ftpsync");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "ftpsync failed:\n{combined}");
    combined
}

/// A scratch directory holding the local source tree, deleted on drop.
pub struct LocalTree {
    path: PathBuf,
}

impl LocalTree {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ftpsync-it-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).expect("could not create the local tree");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, rel: &str, contents: &[u8]) {
        let dest = self.path.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).expect("could not create a local directory");
        }
        std::fs::write(dest, contents).expect("could not write a local file");
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path.join(rel)).expect("could not remove a local file");
    }
}

impl Drop for LocalTree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Deterministic pseudo-random bytes, to exercise binary transfers without
/// pulling in an RNG crate.
pub fn binary_blob(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}
