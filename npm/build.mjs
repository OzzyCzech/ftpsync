#!/usr/bin/env node
// Assemble and publish the npm packages for a tagged release.
//
// For each supported target it downloads the release archive built by the
// `upload-assets` job, extracts the binary into a per-platform package
// (`@ozzyczech/ftpsync-<os>-<cpu>`, gated by `os`/`cpu`), then publishes those
// packages plus the main `@ozzyczech/ftpsync` launcher (whose
// optionalDependencies pull in exactly one matching platform package).
//
// Usage: node npm/build.mjs <version|vX.Y.Z>
// Env:   GH_TOKEN (gh release download). Publishing uses npm Trusted Publishing
//        (OIDC) — no NODE_AUTH_TOKEN; `npm publish --provenance` authenticates
//        via the workflow's id-token.

import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  writeFileSync,
  copyFileSync,
  rmSync,
  readFileSync,
  chmodSync,
} from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url)); // the npm/ directory
const mainSrc = join(root, "ftpsync");
const dist = join(root, "_dist");

const SCOPE = "@ozzyczech";
const TARGETS = [
  { rust: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64", ext: "tar.gz" },
  { rust: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64", ext: "tar.gz" },
  { rust: "x86_64-apple-darwin", os: "darwin", cpu: "x64", ext: "tar.gz" },
  { rust: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", ext: "tar.gz" },
  { rust: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", ext: "zip" },
];

const versionArg = process.argv[2];
if (!versionArg) {
  console.error("usage: node npm/build.mjs <version|vX.Y.Z>");
  process.exit(1);
}
const version = versionArg.replace(/^v/, "");
const tag = `v${version}`;

function run(cmd, args, opts = {}) {
  execFileSync(cmd, args, { stdio: "inherit", ...opts });
}

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

const optionalDependencies = {};
const packages = []; // { dir, name }

// True if this exact name@version is already on the registry (makes re-runs safe).
function versionExists(name) {
  try {
    const out = execFileSync("npm", ["view", `${name}@${version}`, "version"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    return out === version;
  } catch {
    return false;
  }
}

for (const t of TARGETS) {
  const exe = t.os === "win32" ? "ftpsync.exe" : "ftpsync";
  const archive = `ftpsync-${t.rust}.${t.ext}`;
  const dl = join(dist, "_dl", t.rust);
  mkdirSync(dl, { recursive: true });

  run("gh", ["release", "download", tag, "--pattern", archive, "--dir", dl, "--clobber"]);
  if (t.ext === "tar.gz") {
    run("tar", ["xzf", join(dl, archive), "-C", dl]);
  } else {
    run("unzip", ["-o", join(dl, archive), "-d", dl]);
  }

  const name = `${SCOPE}/ftpsync-${t.os}-${t.cpu}`;
  const pkgDir = join(dist, `ftpsync-${t.os}-${t.cpu}`);
  mkdirSync(join(pkgDir, "bin"), { recursive: true });
  copyFileSync(join(dl, exe), join(pkgDir, "bin", exe));
  if (t.os !== "win32") {
    chmodSync(join(pkgDir, "bin", exe), 0o755);
  }

  writeFileSync(
    join(pkgDir, "package.json"),
    JSON.stringify(
      {
        name,
        version,
        description: `ftpsync prebuilt binary for ${t.os}-${t.cpu}`,
        repository: {
          type: "git",
          url: "git+https://github.com/OzzyCzech/ftpsync.git",
        },
        license: "MIT",
        os: [t.os],
        cpu: [t.cpu],
        files: ["bin"],
      },
      null,
      2,
    ) + "\n",
  );

  optionalDependencies[name] = version;
  packages.push({ dir: pkgDir, name });
  console.log(`assembled ${name}@${version}`);
}

// Main launcher package: copy the shim + README, pin versions.
const mainDir = join(dist, "ftpsync");
mkdirSync(join(mainDir, "bin"), { recursive: true });
copyFileSync(join(mainSrc, "bin", "ftpsync.js"), join(mainDir, "bin", "ftpsync.js"));
copyFileSync(join(mainSrc, "README.md"), join(mainDir, "README.md"));

const mainPkg = JSON.parse(readFileSync(join(mainSrc, "package.json"), "utf8"));
mainPkg.version = version;
mainPkg.optionalDependencies = optionalDependencies;
writeFileSync(join(mainDir, "package.json"), JSON.stringify(mainPkg, null, 2) + "\n");
packages.push({ dir: mainDir, name: mainPkg.name });

// Provenance attestation is only available from a supported CI (Sigstore OIDC);
// a local bootstrap publish must omit it.
const provenance = process.env.GITHUB_ACTIONS === "true" ? ["--provenance"] : [];

// Publish platform packages first so the main package's deps resolve immediately.
// Already-published versions are skipped so re-runs and partial failures are safe.
let published = 0;
for (const { dir, name } of packages) {
  if (versionExists(name)) {
    console.log(`skip ${name}@${version} (already published)`);
    continue;
  }
  run("npm", ["publish", "--access", "public", ...provenance], { cwd: dir });
  published++;
}
console.log(`published ${published} package(s), skipped ${packages.length - published}, at ${version}`);
