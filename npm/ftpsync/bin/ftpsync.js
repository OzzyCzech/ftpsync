#!/usr/bin/env node
"use strict";

// Thin launcher: resolve the prebuilt binary from the matching platform package
// (installed via optionalDependencies) and exec it, forwarding args/stdio/exit code.

const { spawnSync } = require("node:child_process");

function resolveBinary() {
  const { platform, arch } = process;
  const pkg = `@ozzyczech/ftpsync-${platform}-${arch}`;
  const exe = platform === "win32" ? "ftpsync.exe" : "ftpsync";
  try {
    return require.resolve(`${pkg}/bin/${exe}`);
  } catch {
    throw new Error(
      `ftpsync: no prebuilt binary for ${platform}-${arch}.\n` +
        `Expected the optional dependency "${pkg}" to be installed.\n` +
        `If your platform is unsupported, build from source: https://github.com/OzzyCzech/ftpsync`,
    );
  }
}

let binary;
try {
  binary = resolveBinary();
} catch (err) {
  console.error(err.message);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`ftpsync: failed to launch binary: ${result.error.message}`);
  process.exit(1);
}
// Re-raise the binary's terminating signal, otherwise exit with its code.
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 0);
}
