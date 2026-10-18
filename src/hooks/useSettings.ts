import { useCallback, useEffect, useState } from "react";

import { errorMessage, getSettings, updateSettings } from "../lib/ipc";
import type { Settings } from "../lib/types";

export interface SettingsState {
  settings: Settings | null;
  error: string | null;
  /** True once the first load settles, either way. */
  ready: boolean;
  patch: (delta: Partial<Settings>) => Promise<Settings>;
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

  const patch = useCallback(async (delta: Partial<Settings>) => {
    // The backend returns the full, clamped settings, so local state cannot drift
    // away from what was actually stored.
    const next = await updateSettings(delta);
    setSettings(next);
    setError(null);
    return next;
  }, []);

  const clearError = useCallback(() => setError(null), []);

  return { settings, error, ready, patch, clearError };
}
