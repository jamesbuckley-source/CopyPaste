#!/usr/bin/env node
// Opens the CopyPaste feature board outside Copilot chat.
//
// The board is normally served by the Copilot canvas extension, which only runs
// while a canvas panel is open. This launcher starts the same server from
// `.github/extensions/copypaste-feature-tracker/server.mjs` and opens it in the
// default browser, so the board is reachable without asking the agent.
//
// Usage: node scripts/feature-board.mjs [--no-open] [--status planned] [--query text]

import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { startInstanceServer } from "../.github/extensions/copypaste-feature-tracker/server.mjs";
import { STATUSES } from "../.github/extensions/copypaste-feature-tracker/store.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const workspaceRoot = path.resolve(scriptDir, "..");

function parseArgs(argv) {
  const options = { open: true, view: {} };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--no-open") {
      options.open = false;
    } else if (arg === "--status") {
      const value = argv[++index];
      if (!STATUSES.includes(value)) {
        throw new Error(`--status must be one of: ${STATUSES.join(", ")}`);
      }
      options.view.status = value;
    } else if (arg === "--query") {
      options.view.query = argv[++index] ?? "";
    } else if (arg === "--help" || arg === "-h") {
      options.help = true;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return options;
}

function openInBrowser(url) {
  // `start` is a cmd builtin, so it has to run through cmd.exe. The empty title
  // argument keeps `start` from treating the URL as a window title.
  if (process.platform === "win32") {
    spawn(process.env.ComSpec ?? "cmd.exe", ["/c", "start", "", url], {
      detached: true,
      stdio: "ignore",
    }).unref();
    return;
  }
  const opener = process.platform === "darwin" ? "open" : "xdg-open";
  spawn(opener, [url], { detached: true, stdio: "ignore" }).unref();
}

const usage = `CopyPaste feature board

  node scripts/feature-board.mjs [options]

Options
  --status <status>  Open the board filtered to backlog, planned, in-progress,
                     blocked or shipped.
  --query <text>     Start with a search term.
  --no-open          Print the URL without launching a browser.
  -h, --help         Show this message.`;

let options;
try {
  options = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`${error.message}\n\n${usage}`);
  process.exit(2);
}

if (options.help) {
  console.log(usage);
  process.exit(0);
}

const server = await startInstanceServer({ workspaceRoot, view: options.view });

console.log(`CopyPaste feature board: ${server.url}`);
console.log(`Artifact: ${server.artifactPath}`);
console.log("Press Ctrl+C to stop.");

if (options.open) openInBrowser(server.url);

let closing = false;
async function shutdown(signal) {
  if (closing) return;
  closing = true;
  console.log(`\n${signal} received; stopping the board.`);
  await server.close();
  process.exit(0);
}

process.on("SIGINT", () => void shutdown("SIGINT"));
process.on("SIGTERM", () => void shutdown("SIGTERM"));
