# ftpsync

[![NPM Downloads](https://img.shields.io/npm/dm/@ozzyczech/ftpsync?style=for-the-badge)](https://www.npmjs.com/package/@ozzyczech/ftpsync)
[![NPM Version](https://img.shields.io/npm/v/@ozzyczech/ftpsync?style=for-the-badge)](https://www.npmjs.com/package/@ozzyczech/ftpsync)
[![NPM License](https://img.shields.io/npm/l/@ozzyczech/ftpsync?style=for-the-badge)](https://github.com/OzzyCzech/ftpsync/blob/main/LICENSE)
[![Last Commit](https://img.shields.io/github/last-commit/OzzyCzech/ftpsync?style=for-the-badge)](https://github.com/OzzyCzech/ftpsync/commits/main)
[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/OzzyCzech/ftpsync/ci.yml?style=for-the-badge)](https://github.com/OzzyCzech/ftpsync/actions)

**Hash-based deploy over FTPS — no SSH, no mtime/size guessing.**

`ftpsync` syncs a local directory to an FTP(S) server by comparing **SHA-256
content hashes**, so only genuinely changed files are uploaded. It keeps a small
JSON state file on the server (`.ftpsync-state.json`) recording the hash of every
deployed file. A single static binary, nothing to install on the target — ideal
for CI/CD pipelines deploying to cheap shared hosting that only offers FTP.

## Features

- **Content-hash diffing** — SHA-256 of file contents, never mtime/size, so a
  `git checkout` or rebuild won't re-upload unchanged files.
- **Auto-init** — on the first run against a populated server, it lists, downloads
  and hashes the existing files to build the initial state (no full re-upload).
- **Parallel uploads** — configurable connection pool (`-j`).
- **Atomic uploads** — files are sent to `{path}.ftpsync-tmp` then renamed onto
  the target, so a half-uploaded file never replaces a live one.
- **`.ftpignore`** — gitignore-style filtering, plus `--include`/`--exclude` globs.
- **FTPS by default** — explicit AUTH TLS via [rustls](https://github.com/rustls/rustls)
  (no system OpenSSL); `--insecure-tls` for self-signed certs.
- **Safe state handling** — size cap (100 MB), schema + version checks, and
  path-traversal rejection. Paths containing control characters (which could
  inject commands on the FTP control channel) are refused outright.

## Installation

### npm

The binary is also published to npm; only the prebuilt binary for your platform
is downloaded (via per-platform `optionalDependencies`, no post-install step):

```bash
npm install -g @ozzyczech/ftpsync
# or run on demand:
npx @ozzyczech/ftpsync --help
```

### Pre-built binaries

Download the archive for your platform from the
[latest release](https://github.com/OzzyCzech/ftpsync/releases/latest), extract,
and put `ftpsync` on your `PATH`:

```bash
curl -sSL https://github.com/OzzyCzech/ftpsync/releases/latest/download/ftpsync-x86_64-unknown-linux-musl.tar.gz | tar xz
sudo mv ftpsync /usr/local/bin/
ftpsync --version
```

### From source (cargo)

```bash
cargo install --git https://github.com/OzzyCzech/ftpsync
```

### Build locally

```bash
git clone https://github.com/OzzyCzech/ftpsync
cd ftpsync
cargo build --release        # -> target/release/ftpsync
```

For a fully static Linux binary (Alpine / `scratch` images):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

## Quick start

```bash
# Deploy the current directory to /www on the server
ftpsync \
  --server ftp.example.com \
  --username deploy \
  --password 's3cret' \
  --server-dir /www
```

Prefer the `FTPSYNC_PASSWORD` environment variable so the password never appears
in your shell history or process list:

```bash
export FTPSYNC_PASSWORD='s3cret'
ftpsync -s ftp.example.com -u deploy -r /www
```

Always **preview first** with `--dry-run`:

```bash
ftpsync -s ftp.example.com -u deploy -r /www --dry-run -v
```

## Usage

```
ftpsync [OPTIONS] --server <SERVER> --username <USERNAME> --password <PASSWORD>
```

### Required

| Option | Description |
|---|---|
| `-s, --server <HOST>` | FTP server hostname |
| `-u, --username <USER>` | FTP username |
| `-p, --password <PASS>` | FTP password (or set `FTPSYNC_PASSWORD`) |

### Connection

| Option | Default | Description |
|---|---|---|
| `--port <PORT>` | `21` | FTP port |
| `--secure <MODE>` | `explicit` | `none` \| `explicit` \| `implicit` |
| `--insecure-tls` | off | Skip TLS certificate validation (self-signed certs) |
| `--passive <BOOL>` | `true` | Passive mode |
| `--timeout <SEC>` | `30` | Connection/handshake timeout |

### Paths

| Option | Default | Description |
|---|---|---|
| `-l, --local-dir <DIR>` | `.` | Local source directory |
| `-r, --server-dir <DIR>` | `/` | Remote target directory |
| `--state-file <NAME>` | `.ftpsync-state.json` | State file name on the server |

### Filters

| Option | Description |
|---|---|
| `--include <GLOB>` | Glob to include (repeatable → whitelist mode) |
| `--exclude <GLOB>` | Glob to exclude (repeatable) |
| `--ignore-file <FILE>` | Path to `.ftpignore` (default `.ftpignore`) |
| `--no-ignore-file` | Don't read `.ftpignore` |

### Behavior

| Option | Description |
|---|---|
| `--no-auto-init` | Treat the server as empty on first run (upload everything). By default ftpsync hashes every remote file on first run to bootstrap state |
| `--no-delete` | Don't delete remote files that are missing locally |
| `--purge <DIR>` | Empty a remote directory after deploying, e.g. a cache (repeatable; the directory itself is kept). Local files inside a purge dir are skipped, not uploaded |
| `--file-perms <OCTAL>` | chmod uploaded files, e.g. `0644` (best-effort via `SITE CHMOD`) |
| `--dir-perms <OCTAL>` | chmod created directories, e.g. `0755` (best-effort via `SITE CHMOD`) |
| `-j, --concurrency <N>` | Parallel uploads (default `4`) |
| `--dry-run` | Print actions without executing them |
| `-v, --verbose` / `-q, --quiet` | More / less output |

## Examples

```bash
# Static site: deploy only the build output
ftpsync -s ftp.example.com -u deploy -r /www --include 'dist/**'

# Deploy a single subdirectory to a matching remote path
ftpsync -s ftp.example.com -u deploy \
        --local-dir build/theme \
        --server-dir /www/theme

# Exclude directories you don't manage
ftpsync -s ftp.example.com -u deploy -r /www \
        --exclude 'vendor/**' --exclude 'uploads/**'

# Empty a cache directory after deploying, and set file/dir permissions
ftpsync -s ftp.example.com -u deploy -r /www \
        --purge cache/views --file-perms 0644 --dir-perms 0755

# Self-signed certificate (e.g. some Czech shared hosts)
ftpsync -s ftp.example.com -u deploy -r /www --insecure-tls

# Faster deploy with more parallel connections
ftpsync -s ftp.example.com -u deploy -r /www -j 8
```

## `.ftpignore`

Gitignore syntax, read from `--local-dir` by default:

```gitignore
node_modules/
*.log
!important.log
.git/
.env*
.DS_Store
```

## State file

`ftpsync` stores `.ftpsync-state.json` in the remote `--server-dir`. Paths are
POSIX and relative to `--server-dir`; hashes are SHA-256 of file contents. The
format is **shared with the Bun implementation** so either tool can read the other's
state:

```json
{
  "version": 1,
  "tool": "ftpsync 0.1.1",
  "updated": "2026-06-02T15:00:00Z",
  "files": {
    "index.html": {
      "hash": "sha256:abc123…",
      "size": 4096,
      "uploaded": "2026-06-02T15:00:00Z"
    }
  }
}
```

> **Auto-init cost:** the first run against a server without a state file downloads
> and hashes every remote file to build the baseline. For large sites (e.g. a full
> WordPress install) this can take a while — use `--no-auto-init` to skip it and
> upload everything instead.

## How it works

1. **Discover** local files (`--include`/`--exclude` + `.ftpignore`).
2. **Hash** every local file with streaming SHA-256.
3. **Connect** over FTPS and fetch `.ftpsync-state.json`.
4. **Auto-init** if no state exists: list + download + hash remote files.
5. **Diff** local hashes against the state → uploads (changed/new) and deletes
   (present in state, missing locally).
6. **Execute** uploads in parallel (atomic temp + rename) and deletes.
7. **Commit** the refreshed state file back to the server.

## Use in CI/CD

### GitHub Actions

```yaml
deploy:
  runs-on: ubuntu-latest
  if: github.ref == 'refs/heads/main'
  steps:
    - uses: actions/checkout@v6
    - name: Install ftpsync
      run: |
        curl -sSL https://github.com/OzzyCzech/ftpsync/releases/latest/download/ftpsync-x86_64-unknown-linux-musl.tar.gz | tar xz
        sudo mv ftpsync /usr/local/bin/
    - name: Deploy
      env:
        FTPSYNC_PASSWORD: ${{ secrets.FTP_PASSWORD }}
      run: ftpsync -s "${{ secrets.FTP_HOST }}" -u "${{ secrets.FTP_USER }}" -r /www -j 8
```

### GitLab CI

```yaml
deploy:production:
  image: alpine:3.20
  rules:
    - if: '$CI_COMMIT_BRANCH == "main"'
  before_script:
    - wget -qO- https://github.com/OzzyCzech/ftpsync/releases/latest/download/ftpsync-x86_64-unknown-linux-musl.tar.gz | tar xz -C /usr/local/bin
  script:
    - ftpsync --server "$FTP_HOST" --username "$FTP_USER" --server-dir /www --concurrency 8
  variables:
    FTPSYNC_PASSWORD: "$FTP_PASSWORD"
```

## Development

```bash
cargo fmt           # format
cargo clippy --all-targets -- -D warnings   # lint (CI is strict)
cargo test          # unit tests
cargo build --release
```

Tests cover hashing, state (de)serialization + path-traversal guards, the
walker/ignore filters, config validation, and `LIST`-line parsing.

### Releasing

Pushing a `vX.Y.Z` tag triggers `.github/workflows/release.yml`, which:

1. creates the GitHub release,
2. builds and attaches binaries for all targets (`upload-assets`),
3. assembles and publishes the npm packages (`publish-npm`): one per-platform
   package (`@ozzyczech/ftpsync-<os>-<cpu>`) plus the `@ozzyczech/ftpsync`
   launcher (`npm/build.mjs`).

Publishing uses **npm Trusted Publishing (OIDC)** — no `NPM_TOKEN` secret. The
job authenticates via its `id-token` and publishes with provenance. One-time
setup on npmjs.com: for each package (`@ozzyczech/ftpsync` and the five
`@ozzyczech/ftpsync-<os>-<cpu>`), add a Trusted Publisher pointing at the
`OzzyCzech/ftpsync` repo and the `release.yml` workflow. Keep the version in
`Cargo.toml` in sync with the tag.

`build.mjs` skips any package whose version is already on the registry, so
re-running a release (or recovering from a partial failure) is safe. The very
first publish of a brand-new package name can't use OIDC (a Trusted Publisher
can only be added to an existing package) — bootstrap it once with a local
`npm login` + `node npm/build.mjs <version>`, then configure the publishers.

## Notes & guarantees

- **TLS** via rustls (`futures-rustls`) — no system OpenSSL dependency.
- **Atomic uploads** — temp file + rename, never a half-written live file.
- **Robust downloads** — verified against the server-reported `SIZE` and retried
  with backoff + reconnect. Some FTP servers race the data-channel close against
  the `226` completion reply, which can otherwise yield a silently truncated
  transfer; `ftpsync` detects this and refuses to commit a corrupt state.
- **Passwords** are never logged and read from `FTPSYNC_PASSWORD` when available.
- **Passive NAT workaround** — in passive mode the data channel connects to the
  control host instead of the IP the server advertises in its PASV reply, so
  misconfigured/NATed servers (e.g. advertising `0.0.0.0`) still work.
- **Deploy marker** — a `<state-file>.running` marker is written while a deploy
  mutates the server and removed when it finishes, making an interrupted or
  overlapping run visible. It is advisory only: it surfaces concurrent deploys
  but does not prevent them (the check and write are not atomic over FTP).

This whole project was inspired by [`dg/ftp-deployment`](https://github.com/dg/ftp-deployment) and [`git-ftp`](https://github.com/git-ftp/git-ftp), thank you for your work!

## License

[MIT](LICENSE)
