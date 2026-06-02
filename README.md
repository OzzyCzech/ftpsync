# ftpsync (Rust)

Hash-based deploy over FTPS without SSH. Single static binary, no runtime to
install. Inspired by `dg/ftp-deployment` and `git-ftp`.

## Build

```bash
cargo build --release      # -> target/release/ftpsync
./target/release/ftpsync --help
```

Static musl binary for Alpine/scratch CI images:

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

## How it works

1. Walk the local dir, applying `--include`/`--exclude` and `.ftpignore`.
2. SHA-256 hash every local file (streaming, 64 KB chunks).
3. Connect over FTPS and fetch `.ftpsync-state.json` from the server.
4. If no state exists, **auto-init**: list + download + hash the remote files.
5. Diff local hashes against the state; upload changed/new, delete missing.
6. Upload the refreshed state file.

The state file format is shared with the Bun implementation (`version: 1`,
`files` map of `{ hash, size, uploaded }`).

## Options

See `ftpsync --help`. Highlights:

- `--secure none|explicit|implicit` (default `explicit`)
- `--insecure-tls` for self-signed certs
- `-j, --concurrency N` parallel uploads via a connection pool
- `--no-delete`, `--no-auto-init`, `--dry-run`
- Password may be passed via the `FTPSYNC_PASSWORD` env var instead of `-p`.

## Tests

```bash
cargo test
```

Unit tests cover hashing, state (de)serialization + path-traversal guards,
the walker/ignore filters, config validation, and LIST-line parsing.

## Notes

- TLS via **rustls** (`futures-rustls`) — no system OpenSSL dependency.
- Uploads are atomic: written to `{path}.ftpsync-tmp`, then renamed onto the
  target.
- Downloads are verified against the server-reported `SIZE` and retried with
  backoff + reconnect: some FTP servers race the data-channel close against the
  `226` completion reply, which can otherwise yield a silently truncated
  transfer.
- The state file is validated (max 100 MB, schema, path-traversal check).
