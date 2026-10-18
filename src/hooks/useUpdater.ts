/**
 * Auto-update wiring.
 *
 * The updater plugin talks to GitHub Releases: `latest.json` names the newest
 * signed installer, and the plugin verifies its signature with the public key
 * baked into the app. The schedule is deliberately lazy — a check runs when the
 * stored timestamp is more than a day old, and an hourly timer only picks it up
 * again once another day has passed.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

const DAY_MS = 24 * 60 * 60 * 1000;
const HOUR_MS = 60 * 60 * 1000;

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "available"; version: string; notes: string | null }
  | { kind: "downloading"; version: string; percent: number | null }
  | { kind: "error"; message: string };

export interface UpdaterControls {
  state: UpdateState;
  checkNow: () => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

export function useUpdater(options: {
  enabled: boolean;
  lastCheck: number;
  /** Records that a check happened, so the daily schedule survives restarts. */
  onChecked: () => void;
  onNotice: (message: string) => void;
}): UpdaterControls {
  const { enabled, lastCheck, onChecked, onNotice } = options;
  const [state, setState] = useState<UpdateState>({ kind: "idle" });
  const pending = useRef<Update | null>(null);
  const busy = useRef(false);

  const checkNow = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    setState({ kind: "checking" });
    try {
      const found = await check();
      onChecked();
      if (found) {
        pending.current = found;
        setState({ kind: "available", version: found.version, notes: found.body ?? null });
      } else {
        pending.current = null;
        setState({ kind: "idle" });
        onNotice("CopyPaste is up to date.");
      }
    } catch (e) {
      // Being offline is the normal case, so a scheduled check stays quiet.
      setState({ kind: "idle" });
      onNotice(`Could not check for updates: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      busy.current = false;
    }
  }, [onChecked, onNotice]);

  useEffect(() => {
    if (!enabled) return;
    if (Date.now() - lastCheck > DAY_MS) {
      void checkNow();
    }
    const timer = window.setInterval(() => {
      if (Date.now() - lastCheck > DAY_MS) void checkNow();
    }, HOUR_MS);
    return () => window.clearInterval(timer);
  }, [enabled, lastCheck, checkNow]);

  const install = useCallback(async () => {
    const update = pending.current;
    if (!update) return;

    const version = update.version;
    setState({ kind: "downloading", version, percent: null });
    try {
      let total = 0;
      let received = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          const percent = total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null;
          setState({ kind: "downloading", version, percent });
        }
      });
      await relaunch();
    } catch (e) {
      setState({
        kind: "error",
        message: `The update could not be installed: ${e instanceof Error ? e.message : String(e)}`,
      });
    }
  }, []);

  const dismiss = useCallback(() => {
    setState({ kind: "idle" });
  }, []);

  return { state, checkNow, install, dismiss };
}
