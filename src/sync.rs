//! Diff computation + execution (connect, auto-init, parallel uploads, commit).

use crate::client::Client;
use crate::config::{Config, Verbosity};
use crate::error::{FtpSyncError, Result};
use crate::hasher;
use crate::state::State;
use crate::walker::{self, LocalFile};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The plan of work derived from the local/remote diff.
#[derive(Debug, Default)]
struct Plan {
    to_upload: Vec<LocalFile>,
    to_delete: Vec<String>,
}

/// Top-level entry point invoked from main.
pub async fn run(cfg: Config) -> Result<()> {
    let verbosity = cfg.verbosity;

    // 1. Discover + hash local files.
    let local = walker::discover(&cfg)?;
    log(
        verbosity,
        Verbosity::Verbose,
        &format!("Discovered {} local files", local.len()),
    );

    let mut local_hashes: HashMap<String, (String, u64)> = HashMap::new();
    for f in &local {
        let hash = hasher::hash_file(&f.abs_path)?;
        let size = std::fs::metadata(&f.abs_path)?.len();
        local_hashes.insert(f.rel_path.clone(), (hash, size));
    }

    // 2. Connect + fetch (or initialize) state.
    log(
        verbosity,
        Verbosity::Normal,
        &format!("Connecting to {}:{}", cfg.server, cfg.port),
    );
    let mut client = Client::connect_and_login(&cfg).await?;

    let state_path = cfg.remote_path(&cfg.state_file);
    let state = match client.download(&state_path).await {
        Ok(bytes) => {
            log(verbosity, Verbosity::Verbose, "Loaded existing state file");
            State::from_bytes(&bytes)?
        }
        Err(FtpSyncError::NotFound(_)) if cfg.auto_init => {
            log(
                verbosity,
                Verbosity::Normal,
                "No state file found — auto-initializing from server (this can be slow)",
            );
            auto_init(&mut client, &cfg, verbosity).await?
        }
        Err(FtpSyncError::NotFound(_)) => {
            log(
                verbosity,
                Verbosity::Normal,
                "No state file found — treating server as empty",
            );
            State::empty()
        }
        Err(e) => return Err(e),
    };

    // 3. Diff.
    let plan = diff(&local, &local_hashes, &state, cfg.no_delete);
    log(
        verbosity,
        Verbosity::Normal,
        &format!(
            "{} to upload, {} to delete",
            plan.to_upload.len(),
            plan.to_delete.len()
        ),
    );

    // 4. Dry run: report and stop.
    if cfg.dry_run {
        for f in &plan.to_upload {
            println!("UPLOAD  {}", f.rel_path);
        }
        for p in &plan.to_delete {
            println!("DELETE  {p}");
        }
        client.quit().await?;
        return Ok(());
    }

    if plan.to_upload.is_empty() && plan.to_delete.is_empty() && cfg.purge.is_empty() {
        log(
            verbosity,
            Verbosity::Normal,
            "Nothing to do — server is up to date",
        );
        client.quit().await?;
        return Ok(());
    }

    // 5. Drop a ".running" marker. This is advisory only — it does not prevent
    //    a concurrent deploy (the exists-check and upload are not atomic over
    //    FTP); it just surfaces an interrupted or overlapping run.
    let lock_path = cfg.remote_path(&format!("{}.running", cfg.state_file));
    if client.exists(&lock_path).await? {
        log(
            verbosity,
            Verbosity::Normal,
            &format!(
                "warning: {lock_path} already exists — a previous deploy may have been \
                 interrupted or another is running"
            ),
        );
    }
    client
        .upload(&lock_path, b"ftpsync deploy in progress\n")
        .await?;

    // 6. Mutate the server, then always release the lock (even on error).
    let result = deploy_mutations(
        &mut client,
        &cfg,
        &plan,
        &local_hashes,
        state,
        verbosity,
        &state_path,
    )
    .await;
    let _ = client.delete(&lock_path).await;
    result?;

    client.quit().await?;
    Ok(())
}

/// Run the mutating phase: deletes, parallel uploads, purge, and state commit.
#[allow(clippy::too_many_arguments)]
async fn deploy_mutations(
    client: &mut Client,
    cfg: &Config,
    plan: &Plan,
    local_hashes: &HashMap<String, (String, u64)>,
    mut state: State,
    verbosity: Verbosity,
    state_path: &str,
) -> Result<()> {
    // Deletes on the primary connection.
    for path in &plan.to_delete {
        let remote = cfg.remote_path(path);
        log(verbosity, Verbosity::Verbose, &format!("DELETE {path}"));
        client.delete(&remote).await?;
        state.remove(path);
    }

    // Uploads (parallel via connection pool).
    let state = Arc::new(Mutex::new(state));
    execute_uploads(
        cfg,
        &plan.to_upload,
        local_hashes,
        Arc::clone(&state),
        verbosity,
    )
    .await?;
    let mut state = Arc::try_unwrap(state)
        .map_err(|_| FtpSyncError::Config("internal: state still shared".into()))?
        .into_inner();

    // Purge requested directories (e.g. caches) after the sync.
    for dir in &cfg.purge {
        let remote = cfg.remote_path(dir);
        log(verbosity, Verbosity::Normal, &format!("Purging {dir}"));
        client.purge(&remote).await?;
    }

    // Commit state.
    let bytes = state.render_json()?;
    client.upload(state_path, &bytes).await?;
    log(verbosity, Verbosity::Normal, "State committed");
    Ok(())
}

/// Compute the upload/delete plan.
fn diff(
    local: &[LocalFile],
    local_hashes: &HashMap<String, (String, u64)>,
    state: &State,
    no_delete: bool,
) -> Plan {
    let mut plan = Plan::default();

    for f in local {
        let local_hash = local_hashes.get(&f.rel_path).map(|(h, _)| h.as_str());
        let remote_hash = state.files.get(&f.rel_path).map(|e| e.hash.as_str());
        if local_hash != remote_hash {
            plan.to_upload.push(f.clone());
        }
    }

    if !no_delete {
        for path in state.files.keys() {
            if !local_hashes.contains_key(path) {
                plan.to_delete.push(path.clone());
            }
        }
    }

    plan
}

/// Auto-init: hash every remote file under server-dir to bootstrap the state.
async fn auto_init(client: &mut Client, cfg: &Config, verbosity: Verbosity) -> Result<State> {
    let mut state = State::empty();
    let remote_files = client
        .list_recursive(&format!("/{}", cfg.server_dir))
        .await?;
    for rel in remote_files {
        if rel == cfg.state_file {
            continue;
        }
        let remote = cfg.remote_path(&rel);
        log(
            verbosity,
            Verbosity::Verbose,
            &format!("HASH (remote) {rel}"),
        );
        let bytes = client.download(&remote).await?;
        let hash = hasher::hash_bytes(&bytes);
        state.set(&rel, hash, bytes.len() as u64);
    }
    // Persist the freshly-built state so subsequent runs skip auto-init.
    let bytes = state.render_json()?;
    client
        .upload(&cfg.remote_path(&cfg.state_file), &bytes)
        .await?;
    Ok(state)
}

/// Upload files in parallel across a pool of independent connections.
async fn execute_uploads(
    cfg: &Config,
    files: &[LocalFile],
    local_hashes: &HashMap<String, (String, u64)>,
    state: Arc<Mutex<State>>,
    verbosity: Verbosity,
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let concurrency = cfg.concurrency.max(1).min(files.len());
    let queue = Arc::new(Mutex::new(files.to_vec()));
    let cfg = Arc::new(cfg.clone());
    let hashes = Arc::new(local_hashes.clone());

    let mut workers = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let queue = Arc::clone(&queue);
        let cfg = Arc::clone(&cfg);
        let hashes = Arc::clone(&hashes);
        let state = Arc::clone(&state);
        workers.push(tokio::spawn(async move {
            // Each worker opens its own connection: login -> upload N -> quit.
            let mut client = Client::connect_and_login(&cfg).await?;
            loop {
                let file = {
                    let mut q = queue.lock().await;
                    q.pop()
                };
                let Some(file) = file else { break };

                let data = tokio::task::spawn_blocking({
                    let path = file.abs_path.clone();
                    move || std::fs::read(path)
                })
                .await
                .map_err(|e| FtpSyncError::Config(format!("join error: {e}")))??;

                let remote = cfg.remote_path(&file.rel_path);
                log(
                    verbosity,
                    Verbosity::Normal,
                    &format!("UPLOAD {}", file.rel_path),
                );
                client.upload_atomic(&remote, &data).await?;

                if let Some((hash, size)) = hashes.get(&file.rel_path) {
                    let mut s = state.lock().await;
                    s.set(&file.rel_path, hash.clone(), *size);
                }
            }
            client.quit().await?;
            Ok::<(), FtpSyncError>(())
        }));
    }

    for w in workers {
        w.await
            .map_err(|e| FtpSyncError::Config(format!("worker join error: {e}")))??;
    }
    Ok(())
}

/// Print `msg` if the current verbosity is at least `level`.
fn log(current: Verbosity, level: Verbosity, msg: &str) {
    let rank = |v: Verbosity| match v {
        Verbosity::Quiet => 0,
        Verbosity::Normal => 1,
        Verbosity::Verbose => 2,
    };
    // Quiet suppresses everything except errors (handled by `?`).
    if current == Verbosity::Quiet {
        return;
    }
    if rank(current) >= rank(level) {
        eprintln!("{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn lf(rel: &str) -> LocalFile {
        LocalFile {
            abs_path: PathBuf::from(format!("/local/{rel}")),
            rel_path: rel.to_string(),
        }
    }

    #[test]
    fn diff_uploads_changed_and_new() {
        let local = vec![lf("a.txt"), lf("b.txt")];
        let mut hashes = HashMap::new();
        hashes.insert("a.txt".to_string(), ("sha256:new".to_string(), 1));
        hashes.insert("b.txt".to_string(), ("sha256:bbb".to_string(), 2));

        let mut state = State::empty();
        state.set("a.txt", "sha256:old".to_string(), 1); // changed
        state.set("c.txt", "sha256:ccc".to_string(), 3); // gone locally

        let plan = diff(&local, &hashes, &state, false);
        let up: Vec<_> = plan.to_upload.iter().map(|f| f.rel_path.clone()).collect();
        assert!(up.contains(&"a.txt".to_string())); // changed
        assert!(up.contains(&"b.txt".to_string())); // new
        assert_eq!(plan.to_delete, vec!["c.txt".to_string()]);
    }

    #[test]
    fn diff_skips_unchanged() {
        let local = vec![lf("a.txt")];
        let mut hashes = HashMap::new();
        hashes.insert("a.txt".to_string(), ("sha256:same".to_string(), 1));
        let mut state = State::empty();
        state.set("a.txt", "sha256:same".to_string(), 1);
        let plan = diff(&local, &hashes, &state, false);
        assert!(plan.to_upload.is_empty());
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn diff_respects_no_delete() {
        let local: Vec<LocalFile> = vec![];
        let hashes = HashMap::new();
        let mut state = State::empty();
        state.set("gone.txt", "sha256:x".to_string(), 1);
        let plan = diff(&local, &hashes, &state, true);
        assert!(plan.to_delete.is_empty());
    }
}
