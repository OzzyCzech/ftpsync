# ftpsync

[![CI](https://github.com/OzzyCzech/ftpsync/actions/workflows/ci.yml/badge.svg)](https://github.com/OzzyCzech/ftpsync/actions/workflows/ci.yml)
[![Release](https://github.com/OzzyCzech/ftpsync/actions/workflows/release.yml/badge.svg)](https://github.com/OzzyCzech/ftpsync/actions/workflows/release.yml)

**Hash-based deploy over FTPS — no SSH, no mtime/size guessing.**

`ftpsync` syncs a local directory to an FTP(S) server by comparing **SHA-256
content hashes**, so only genuinely changed files are uploaded. It keeps a small
JSON state file on the server (`.ftpsync-state.json`) recording the hash of every
deployed file. A single static binary, nothing to install on the target — ideal
for CI/CD pipelines deploying to cheap shared hosting that only offers FTP.

Inspired by [`dg/ftp-deployment`](https://github.com/dg/ftp-deployment) and
[`git-ftp`](https://github.com/git-ftp/git-ftp).

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
  path-traversal rejection.

## Installation

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
| `--auto-init` | Hash remote files on first run (default behavior) |
| `--no-auto-init` | Treat the server as empty on first run (upload everything) |
| `--no-delete` | Don't delete remote files that are missing locally |
| `-j, --concurrency <N>` | Parallel uploads (default `4`) |
| `--dry-run` | Print actions without executing them |
| `-v, --verbose` / `-q, --quiet` | More / less output |

## Examples

```bash
# Static site: deploy only the build output
ftpsync -s ftp.example.com -u deploy -r /www --include 'dist/**'

# WordPress theme only
ftpsync -s ftp.wp.cz -u deploy \
        --local-dir wp-content/themes/laguna \
        --server-dir /www/wp-content/themes/laguna

# Exclude server-managed directories
ftpsync -s ftp.example.com -u deploy -r /www \
        --exclude 'wp-admin/**' --exclude 'wp-includes/**'

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
  "tool": "ftpsync 0.1.0",
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
    - uses: actions/checkout@v4
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

## Notes & guarantees

- **TLS** via rustls (`futures-rustls`) — no system OpenSSL dependency.
- **Atomic uploads** — temp file + rename, never a half-written live file.
- **Robust downloads** — verified against the server-reported `SIZE` and retried
  with backoff + reconnect. Some FTP servers race the data-channel close against
  the `226` completion reply, which can otherwise yield a silently truncated
  transfer; `ftpsync` detects this and refuses to commit a corrupt state.
- **Passwords** are never logged and read from `FTPSYNC_PASSWORD` when available.

## License

MIT
