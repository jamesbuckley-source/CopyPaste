import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { errorMessage, listClips } from "../lib/ipc";
import type { ClipFilter, ClipSummary } from "../lib/types";

/** Emitted by the backend whenever the stored history mutates. */
const HISTORY_CHANGED = "history:changed";

/**
 * One page is fetched at a time. The cap keeps the DOM small enough that plain
 * rendering stays smooth without pulling in a virtualiser.
 */
const PAGE_SIZE = 500;

export interface HistoryState {
  items: ClipSummary[];
  /** Matches irrespective of paging, so the header can say "showing 500 of 2000". */
  total: number;
  loading: boolean;
  error: string | null;
  reload: () => Promise<void>;
}

export function useHistory(
  filter: ClipFilter,
  query: string,
  /** Ordering key; changing it must re-fetch because the backend sorts. */
  sortOrder: string,
): HistoryState {
  const [items, setItems] = useState<ClipSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // The filter is an object literal at every call site, so depend on its value
  // rather than its identity.
  const filterKey = JSON.stringify(filter);
  const filterRef = useRef(filter);
  filterRef.current = filter;

  // Monotonic request id: search-as-you-type can have several responses in
  // flight, and an older one must never overwrite a newer result.
  const requestId = useRef(0);

  const reload = useCallback(async () => {
    const token = ++requestId.current;
    try {
      const page = await listClips({
        filter: filterRef.current,
        query,
        limit: PAGE_SIZE,
      });
      if (token !== requestId.current) return;
      setItems(page.items);
      setTotal(page.total);
      setError(null);
    } catch (e) {
      if (token !== requestId.current) return;
      setError(errorMessage(e));
    } finally {
      if (token === requestId.current) setLoading(false);
    }
  }, [filterKey, query, sortOrder]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    const pending = listen(HISTORY_CHANGED, () => {
      void reload();
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [reload]);

  return { items, total, loading, error, reload };
}

/** Delays a rapidly changing value; used to keep search off the keystroke path. */
export function useDebounced<T>(value: T, delayMs: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);
  return settled;
}

/** Re-renders on an interval so relative timestamps stay honest while open. */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}
