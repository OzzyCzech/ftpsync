# @ozzyczech/ftpsync

Hash-based deploy over FTPS without SSH — a single static binary, distributed
as a native npm package. Only the binary for your platform is downloaded (via
per-platform `optionalDependencies`); there is no post-install download step.

## Install

```bash
npm install -g @ozzyczech/ftpsync
# or run without installing:
npx @ozzyczech/ftpsync --help
```

The installed command is `ftpsync`.

## Usage

```bash
ftpsync --server ftp.example.com --username deploy --server-dir /www
```

See the full documentation, options, and CI/CD recipes at
**https://github.com/OzzyCzech/ftpsync**.

## Supported platforms

| OS | Arch | Package |
|---|---|---|
| Linux | x64 | `@ozzyczech/ftpsync-linux-x64` |
| Linux | arm64 | `@ozzyczech/ftpsync-linux-arm64` |
| macOS | x64 | `@ozzyczech/ftpsync-darwin-x64` |
| macOS | arm64 | `@ozzyczech/ftpsync-darwin-arm64` |
| Windows | x64 | `@ozzyczech/ftpsync-win32-x64` |

## License

MIT
