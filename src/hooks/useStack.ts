import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { stackList } from "../lib/ipc";
import type { StackEntry } from "../lib/types";

/** Emitted by the backend whenever the paste stack mutates. */
const STACK_CHANGED = "stack:changed";
/** A queued clip's preview moves when it is copied or edited. */
const HISTORY_CHANGED = "history:changed";

export interface StackState {
  items: StackEntry[];
  /** How many items are queued, for the badge. */
  count: number;
  refresh: () => Promise<void>;
}

/**
 * The paste stack, kept in step with the backend.
 *
 * A failure is swallowed rather than shown: the stack is a side surface, and a
 * banner about it would cover the history the user is working in. The next event
 * tries again.
 */
export function useStack(): StackState {
  const [items, setItems] = useState<StackEntry[]>([]);
  const requestId = useRef(0);

  const refresh = useCallback(async () => {
    const token = ++requestId.current;
    try {
      const next = await stackList();
      if (token === requestId.current) setItems(next);
    } catch {
      // Left as it was; see above.
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const pending = [
      listen(STACK_CHANGED, () => void refresh()),
      // Deleting a queued clip drops its row, and copying one changes its preview.
      listen(HISTORY_CHANGED, () => void refresh()),
    ];
    return () => {
      for (const entry of pending) void entry.then((unlisten) => unlisten());
    };
  }, [refresh]);

  return { items, count: items.length, refresh };
}
