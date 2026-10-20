import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { errorMessage, getSettings, updateSettings } from "../lib/ipc";
import type { Settings } from "../lib/types";

export interface SettingsState {
  settings: Settings | null;
  error: string | null;
  /** True once the first load settles, either way. */
  ready: boolean;
  patch: (delta: Partial<Settings>) => Promise<Settings>;
  refresh: () => void;
  clearError: () => void;
}

export function useSettings(): SettingsState {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((loaded) => {
        if (!cancelled) setSettings(loaded);
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      })
      .finally(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // The settings window is a separate webview, so a change made there has to be
  // pushed here or the main window keeps its old theme and layout.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<Settings>("settings:changed", (event) => {
      if (!cancelled) setSettings(event.payload);
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /** Re-reads settings, e.g. when another window may have changed them. */
  const refresh = useCallback(() => {
    getSettings()
      .then(setSettings)
      .catch(() => undefined);
  }, []);

  const patch = useCallback(async (delta: Partial<Settings>) => {
    // The backend returns the full, clamped settings, so local state cannot drift
    // away from what was actually stored.
    const next = await updateSettings(delta);
    setSettings(next);
    setError(null);
    return next;
  }, []);

  const clearError = useCallback(() => setError(null), []);

  return { settings, error, ready, patch, refresh, clearError };
}
