import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { ClipDetailPane } from "./components/ClipDetailPane";
import { ClipList, scrollRowIntoView } from "./components/ClipList";
import { SearchBar } from "./components/SearchBar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { useDebounced, useHistory, useNow } from "./hooks/useHistory";
import { useSettings } from "./hooks/useSettings";
import {
  clearHistory,
  copyClip,
  createCollection,
  deleteClip,
  deleteCollection,
  errorMessage,
  getClip,
  getStats,
  hideWindow,
  listCollections,
  listTags,
  pruneHistory,
  revealDataDir,
  setClipCollections,
  setClipTags,
  setPinned,
} from "./lib/ipc";
import {
  viewFilter,
  viewLabel,
  type ClipDetail,
  type ClipSummary,
  type Collection,
  type SidebarView,
  type Stats,
  type Tag,
} from "./lib/types";

type Route = "history" | "settings";

/** Sent by the backend when the tray or hotkey wants a particular pane shown. */
const ROUTE_EVENT = "ui:route";
const HISTORY_CHANGED = "history:changed";

export default function App() {
  const [route, setRoute] = useState<Route>("history");
  const [view, setView] = useState<SidebarView>({ kind: "all" });
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<ClipDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Bumped after every mutation so derived data is re-fetched. */
  const [dataVersion, setDataVersion] = useState(0);

  const searchRef = useRef<HTMLInputElement | null>(null);
  const { settings, error: settingsError, patch, clearError } = useSettings();
  const now = useNow();

  // Typing stays local; the store query is debounced so a fast typist does not
  // trigger a decrypt-and-score pass on every keystroke.
  const debouncedQuery = useDebounced(query, 70);

  const filter = useMemo(() => viewFilter(view), [view]);
  const { items, total, loading, error: historyError } = useHistory(filter, debouncedQuery);

  const bump = useCallback(() => setDataVersion((version) => version + 1), []);

  // ---- derived data -------------------------------------------------------

  useEffect(() => {
    const load = () => {
      listCollections().then(setCollections).catch(() => undefined);
      listTags().then(setTags).catch(() => undefined);
      getStats().then(setStats).catch(() => undefined);
    };
    load();
    // Sidebar counts must track captures, not just edits made in this window.
    const pending = listen(HISTORY_CHANGED, load);
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [dataVersion]);

  // Keep a selection at all times so the list is immediately usable with arrows.
  useEffect(() => {
    if (items.length === 0) {
      setSelectedId(null);
      return;
    }
    setSelectedId((current) =>
      current !== null && items.some((item) => item.id === current) ? current : items[0].id,
    );
  }, [items]);

  useEffect(() => {
    if (selectedId === null) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setLoadingDetail(true);
    getClip(selectedId)
      .then((loaded) => {
        if (!cancelled) setDetail(loaded);
      })
      .catch(() => {
        if (!cancelled) setDetail(null);
      })
      .finally(() => {
        if (!cancelled) setLoadingDetail(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, dataVersion]);

  const selectedClip = useMemo(
    () => items.find((item) => item.id === selectedId) ?? null,
    [items, selectedId],
  );

  // ---- window behaviour ---------------------------------------------------

  useEffect(() => {
    const pending = listen<string>(ROUTE_EVENT, (event) => {
      const next: Route = event.payload === "settings" ? "settings" : "history";
      setRoute(next);
      if (next === "history") {
        setQuery("");
        // The window is summoned to be typed into, so put the caret in search.
        window.setTimeout(() => searchRef.current?.focus(), 0);
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    const pending = appWindow.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        if (route === "history") searchRef.current?.focus();
      } else if (settings?.hideOnBlur) {
        void appWindow.hide();
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [route, settings?.hideOnBlur]);

  useEffect(() => {
    if (selectedId !== null) scrollRowIntoView(selectedId);
  }, [selectedId]);

  // ---- actions ------------------------------------------------------------

  const activate = useCallback(async (clip: ClipSummary) => {
    try {
      // The backend decides whether to also paste, and hides the window either way.
      await copyClip(clip.id);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  const removeClip = useCallback(
    async (clip: ClipSummary) => {
      try {
        await deleteClip(clip.id);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const togglePin = useCallback(
    async (clip: ClipSummary) => {
      try {
        await setPinned(clip.id, !clip.pinned);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const commitTags = useCallback(
    async (clip: ClipSummary, next: string[]) => {
      try {
        await setClipTags(clip.id, next);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const commitCollections = useCallback(
    async (clip: ClipSummary, ids: number[]) => {
      try {
        await setClipCollections(clip.id, ids);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const addCollection = useCallback(
    async (name: string) => {
      try {
        const created = await createCollection(name);
        bump();
        setView({ kind: "collection", id: created.id, name: created.name });
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const removeCollection = useCallback(
    async (collection: Collection) => {
      try {
        // Only the grouping goes away; the clips themselves are untouched.
        await deleteCollection(collection.id);
        if (view.kind === "collection" && view.id === collection.id) {
          setView({ kind: "all" });
        }
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, view],
  );

  const runClear = useCallback(
    async (keepPinned: boolean) => {
      try {
        const removed = await clearHistory(keepPinned);
        bump();
        setNotice(`Removed ${removed} clip${removed === 1 ? "" : "s"}.`);
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const runPrune = useCallback(async () => {
    try {
      const removed = await pruneHistory();
      bump();
      setNotice(
        removed === 0 ? "Nothing to prune." : `Pruned ${removed} clip${removed === 1 ? "" : "s"}.`,
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [bump]);

  const openDataDir = useCallback(() => {
    revealDataDir().catch((e) => setError(errorMessage(e)));
  }, []);

  // ---- keyboard -----------------------------------------------------------

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      const inSearch = target === searchRef.current;

      if (event.key === "Escape") {
        event.preventDefault();
        if (inSearch && query) {
          setQuery("");
          return;
        }
        if (route === "settings") {
          setRoute("history");
          return;
        }
        void hideWindow();
        return;
      }

      // Keystrokes in a real form control stay there; only the search field doubles
      // as list navigation.
      if ((tag === "INPUT" || tag === "TEXTAREA") && !inSearch) return;

      const index = items.findIndex((item) => item.id === selectedId);
      const move = (delta: number) => {
        if (items.length === 0) return;
        const next = index < 0 ? 0 : Math.min(items.length - 1, Math.max(0, index + delta));
        setSelectedId(items[next].id);
      };

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          move(1);
          return;
        case "ArrowUp":
          event.preventDefault();
          move(-1);
          return;
        case "PageDown":
          event.preventDefault();
          move(10);
          return;
        case "PageUp":
          event.preventDefault();
          move(-10);
          return;
        case "Home":
          if (items.length > 0) {
            event.preventDefault();
            setSelectedId(items[0].id);
          }
          return;
        case "End":
          if (items.length > 0) {
            event.preventDefault();
            setSelectedId(items[items.length - 1].id);
          }
          return;
        case "Enter":
          if (!selectedClip) return;
          event.preventDefault();
          void activate(selectedClip);
          return;
        case "Delete":
          if (!selectedClip) return;
          event.preventDefault();
          void removeClip(selectedClip);
          return;
        default:
          break;
      }

      // Ctrl+P toggles the pin, matching the affordance offered on each row.
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "p") {
        if (!selectedClip) return;
        event.preventDefault();
        void togglePin(selectedClip);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [items, selectedId, selectedClip, query, route, activate, removeClip, togglePin]);

  // ---- render -------------------------------------------------------------

  const activeFilter = viewLabel(view);
  const emptyMessage = query
    ? `Nothing matches “${query}”.`
    : view.kind === "all"
      ? "Nothing captured yet — copy something and it will appear here."
      : `No clips under ${activeFilter}.`;

  // Failures from any layer surface in one banner.
  const banner = error ?? settingsError ?? historyError;

  return (
    <div className="flex h-full flex-col bg-canvas">
      <TitleBar
        subtitle={
          route === "settings" ? "Settings" : `${activeFilter}${query ? ` · “${query}”` : ""}`
        }
        settingsOpen={route === "settings"}
        onToggleSettings={() => setRoute(route === "settings" ? "history" : "settings")}
        onHide={() => void hideWindow()}
      />

      {banner && (
        <div
          role="alert"
          className="flex shrink-0 items-center gap-2 border-b border-danger/30 bg-danger/10 px-3 py-1.5 text-[11px] text-danger"
        >
          <span className="min-w-0 flex-1">{banner}</span>
          <button
            type="button"
            onClick={() => {
              setError(null);
              clearError();
            }}
            className="shrink-0 underline underline-offset-2"
          >
            Dismiss
          </button>
        </div>
      )}

      {route === "settings" ? (
        <SettingsPanel
          settings={settings}
          stats={stats}
          error={null}
          notice={notice}
          onPatch={patch}
          onDismissError={() => setError(null)}
          onRevealDataDir={openDataDir}
          onClearHistory={(keepPinned) => void runClear(keepPinned)}
          onPrune={() => void runPrune()}
        />
      ) : (
        <div className="flex min-h-0 flex-1">
          <Sidebar
            view={view}
            onSelect={(next) => {
              setView(next);
              setQuery("");
            }}
            stats={stats}
            collections={collections}
            tags={tags}
            onCreateCollection={(name) => void addCollection(name)}
            onDeleteCollection={(collection) => void removeCollection(collection)}
          />

          <main className="flex min-w-0 flex-1 flex-col">
            <SearchBar
              value={query}
              onChange={setQuery}
              inputRef={searchRef}
              busy={loading}
              resultCount={total}
              activeFilter={activeFilter}
              onClearFilter={() => setView({ kind: "all" })}
            />

            <div className="flex min-h-0 flex-1">
              <ClipList
                items={items}
                selectedId={selectedId}
                loading={loading}
                emptyMessage={emptyMessage}
                now={now}
                onSelect={setSelectedId}
                onActivate={(clip) => void activate(clip)}
                onTogglePin={(clip) => void togglePin(clip)}
                onDelete={(clip) => void removeClip(clip)}
              />

              <ClipDetailPane
                clip={selectedClip}
                detail={detail}
                loadingDetail={loadingDetail}
                collections={collections}
                now={now}
                onCopy={(clip) => void activate(clip)}
                onTogglePin={(clip) => void togglePin(clip)}
                onDelete={(clip) => void removeClip(clip)}
                onTagsCommit={(clip, next) => void commitTags(clip, next)}
                onCollectionsChange={(clip, ids) => void commitCollections(clip, ids)}
              />
            </div>
          </main>
        </div>
      )}

      <StatusBar stats={stats} settings={settings} shown={items.length} total={total} />
    </div>
  );
}
