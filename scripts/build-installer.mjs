#!/usr/bin/env node
// Builds the NSIS installer with its updater signature.
//
// `npm run tauri build` alone fails with "a public key has been found, but no
// private key", because the updater key pair deliberately lives outside the
// repository. It sits beside the other Tauri keys — `~/.tauri/copypaste.key`,
// its `.password` sibling, and the `.pub` that tauri.conf.json already holds —
// and is exported here for the build process only, so it never lands in the
// source tree or the shell's persistent environment. CI passes the same values
// as the TAURI_SIGNING_PRIVATE_KEY secrets instead.
//
// Usage: node scripts/build-installer.mjs [extra `tauri build` arguments]

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const keyDir = path.join(os.homedir(), ".tauri");
const keyPath = path.join(keyDir, "copypaste.key");
const passwordPath = path.join(keyDir, "copypaste.password");

function read(file) {
  return readFileSync(file, "utf8").trim();
}

if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  if (!existsSync(keyPath)) {
    console.error(
      `No updater key at ${keyPath}.\n` +
        "Copy it there, or set TAURI_SIGNING_PRIVATE_KEY to the key's contents.",
    );
    process.exit(1);
  }
  process.env.TAURI_SIGNING_PRIVATE_KEY = read(keyPath);
}

if (!process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD && existsSync(passwordPath)) {
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = read(passwordPath);
}

const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const build = spawn(npm, ["run", "tauri", "build", ...process.argv.slice(2)], {
  stdio: "inherit",
  // Node refuses to spawn a `.cmd` shim without a shell on Windows.
  shell: process.platform === "win32",
});

build.on("exit", (code, signal) => {
  process.exit(code ?? (signal ? 1 : 0));
});
