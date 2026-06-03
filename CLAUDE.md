# ftpsync

Hash-based deploy tool over FTPS without SSH. A single static Rust binary that
syncs a local directory to a remote FTP(S) server, uploading only files whose
content hash changed since the last deploy.

## How it works

1. **Discover + hash local files** (`walker.rs`, `hasher.rs`): walk `--local-dir`,
   apply `.ftpignore` / `--include` / `--exclude` filters, then SHA-256 every file.
2. **Fetch remote state** (`state.rs`): download `.ftpsync-state.json` from the
   server — a `{ version, tool, updated, files: { path: { hash, size, uploaded } } }`
   map that records what is currently deployed. On a first run with no state file,
   either hash every remote file to bootstrap state (default) or treat the server
   as empty (`--no-auto-init`).
3. **Diff** (`sync.rs`): compare local hashes against state to produce an
   upload/delete plan. `--dry-run` prints the plan and stops.
4. **Deploy** (`sync.rs`, `client.rs`): drop an advisory `.running` marker, run
   deletes, then parallel uploads (each worker owns its own FTP connection),
   optionally `--purge` cache dirs, and finally commit the new state file.

The state file is the source of truth, so deploys are content-addressed and
idempotent: an interrupted run re-uploads only what didn't make it.

## Module map (`src/`)

- `main.rs` — entry point: parse args → build config → `sync::run`.
- `cli.rs` — clap argument definitions.
- `config.rs` — validated `Config` from args; remote-path helpers; `has_control_chars`.
- `walker.rs` — local file discovery + filtering.
- `ignore.rs` — `.ftpignore` parsing (gitignore semantics via the `ignore` crate).
- `hasher.rs` — streaming SHA-256 (`sha256:<hex>`).
- `client.rs` — async FTPS client (suppaftp + rustls): connect, download (with
  SIZE-verified retry), atomic upload (tmp + rename), list, delete, purge, chmod.
- `state.rs` — state file (de)serialization, with size / version / path-traversal checks.
- `sync.rs` — diff computation and the deploy orchestration.
- `error.rs` — `FtpSyncError` enum + `is_not_found` (550) helper.

## Conventions / invariants

- Remote paths are POSIX, normalized so root = `""` (see `normalize_remote_dir` /
  `join_remote`). State file keys are relative to `--server-dir`.
- Security: paths with control characters are rejected (would inject FTP commands
  on CRLF-terminated control channel); LIST entry names with `/` or control chars
  are dropped; loaded state is checked for `..` / absolute paths.
- The `.running` marker is **advisory only** — it surfaces interrupted/overlapping
  deploys but does not prevent concurrency (exists-check + upload aren't atomic).
- State `BTreeMap` keeps output deterministic. The on-disk format is shared with a
  parallel Bun implementation; keep `version` / shape compatible.

## Build, test, lint

```bash
cargo build              # debug
cargo build --release    # release profile: LTO, 1 codegen unit, stripped
cargo test               # unit tests live in each module under #[cfg(test)]
cargo clippy --all-targets
cargo fmt
```

CI (`.github/workflows/ci.yml`) must stay green: clippy + fmt + tests.

## Distribution

Distributed as an npm package backed by prebuilt binaries (no `cargo install`
needed). The launcher `@ozzyczech/ftpsync` declares per-platform packages
(`@ozzyczech/ftpsync-<os>-<cpu>`) as `optionalDependencies`; npm installs only
the one matching the host's `os`/`cpu`. `npm/ftpsync/bin/ftpsync.js` execs the
bundled binary.

## Releasing a new version

Releases are **git-tag driven** — pushing a `vX.Y.Z` tag triggers
`.github/workflows/release.yml`, which:

1. creates a GitHub Release (notes auto-generated from commits since the last tag),
2. builds and uploads binaries for 6 targets (linux gnu/musl x64+arm64, darwin
   x64+arm64, windows x64),
3. runs `node npm/build.mjs <tag>` to assemble and publish the npm packages via
   npm **Trusted Publishing (OIDC)** — no `NODE_AUTH_TOKEN`; each package needs a
   Trusted Publisher configured on npmjs.com (repo `OzzyCzech/ftpsync`, workflow
   `release.yml`). `build.mjs` derives the version from the tag, overwrites the
   launcher's `version` + `optionalDependencies`, and **skips already-published
   versions** so re-runs / partial failures are safe.

### Steps to cut a release

1. Bump the version in **`Cargo.toml`** (drives the binary's `--version` and the
   `tool` field in the state file) and **`npm/ftpsync/package.json`** (its
   `version` + `optionalDependencies`, kept consistent though `build.mjs`
   regenerates them).
2. `cargo build` to refresh `Cargo.lock`, then commit (`Release vX.Y.Z`) and push `main`.
3. `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. Watch the run: `gh run watch <id> --exit-status` (or `gh run list --workflow=release.yml`).

Do not bump version numbers in the per-platform `optionalDependencies` by hand as
the sole source of truth — the tag is authoritative and `build.mjs` overwrites them.
