import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { ClipDetailPane } from "./components/ClipDetailPane";
import { ClipGrid } from "./components/ClipGrid";
import { ClipList, scrollRowIntoView } from "./components/ClipList";
import { ContextMenu, type ContextMenuItem } from "./components/ContextMenu";
import { SearchBar, type ViewMode } from "./components/SearchBar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { useDebounced, useHistory, useNow } from "./hooks/useHistory";
import { useSettings } from "./hooks/useSettings";
import { useUpdater } from "./hooks/useUpdater";
import {
  clearHistory,
  copyClip,
  copyOcrText,
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
  revealArchive,
  revealClip,
  setClipCollections,
  setClipTags,
  setCollectionArchived,
  setPinned,
} from "./lib/ipc";
import { applyTheme } from "./lib/theme";
import {
  DEFAULT_VIEW_MODES,
  viewFilter,
  viewLabel,
  type ClipDetail,
  type ClipSummary,
  type Collection,
  type Settings,
  type SidebarView,
  type Stats,
  type Tag,
  type ViewModes,
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
  const [appVersion, setAppVersion] = useState<string | null>(null);
  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => undefined);
  }, []);

  /** Applies a settings change, surfacing failures instead of rejecting silently. */
  const applyPatch = useCallback(
    async (delta: Partial<Settings>) => {
      try {
        return await patch(delta);
      } catch (e) {
        setError(errorMessage(e));
        return null;
      }
    },
    [patch],
  );

  // Typing stays local; the store query is debounced so a fast typist does not
  // trigger a decrypt-and-score pass on every keystroke.
  const debouncedQuery = useDebounced(query, 70);

  const filter = useMemo(() => viewFilter(view), [view]);
  const { items, total, loading, error: historyError } = useHistory(filter, debouncedQuery);

  const bump = useCallback(() => setDataVersion((version) => version + 1), []);

  // ---- auto-update --------------------------------------------------------

  const markUpdateChecked = useCallback(() => {
    void applyPatch({ lastUpdateCheck: Date.now() });
  }, [applyPatch]);

  const updater = useUpdater({
    enabled: settings?.autoUpdate ?? false,
    lastCheck: settings?.lastUpdateCheck ?? 0,
    onChecked: markUpdateChecked,
    onNotice: setNotice,
  });

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

  /** Copy from the hover button: the window stays put so browsing continues. */
  const copySilently = useCallback(async (clip: ClipSummary) => {
    try {
      await copyClip(clip.id, false, false);
      setNotice(`Copied “${clip.preview.slice(0, 40) || "clip"}”.`);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  /** Copies what OCR found in an image rather than the bitmap itself. */
  const copyTextFromImage = useCallback(async (clip: ClipSummary) => {
    try {
      await copyOcrText(clip.id, false);
      setNotice("Copied the text recognised in the image.");
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

  const archiveCollection = useCallback(
    async (collection: Collection, archived: boolean) => {
      try {
        await setCollectionArchived(collection.id, archived);
        // An archived collection is no longer part of the ordinary history, so the
        // "All clips" view is the honest place to land after archiving the one in
        // front of you.
        if (archived && view.kind === "collection" && view.id === collection.id) {
          setView({ kind: "all" });
        }
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, view],
  );

  // ---- context menus ------------------------------------------------------

  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    items: ContextMenuItem[];
  } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const openClipMenu = useCallback(
    (clip: ClipSummary, event: ReactMouseEvent) => {
      event.preventDefault();
      const items: ContextMenuItem[] = [];
      if (clip.kind !== "text") {
        items.push({
          label: clip.kind === "image" ? "Show image in Explorer" : "Show in Explorer",
          onSelect: () => {
            revealClip(clip.id).catch((e) => setError(errorMessage(e)));
          },
        });
      }
      if (clip.kind === "image" && clip.hasOcr) {
        items.push({
          label: "Copy text from image",
          onSelect: () => void copyTextFromImage(clip),
        });
      }
      items.push({ label: "Copy", onSelect: () => void copySilently(clip) });
      items.push({
        label: clip.pinned ? "Unpin" : "Pin",
        onSelect: () => void togglePin(clip),
      });
      setMenu({ x: event.clientX, y: event.clientY, items });
    },
    [copySilently, copyTextFromImage, togglePin],
  );

  const openArchiveMenu = useCallback(
    (collection: Collection, event: ReactMouseEvent) => {
      if (!collection.archived) return;
      event.preventDefault();
      setMenu({
        x: event.clientX,
        y: event.clientY,
        items: [
          {
            label: "Show folder in Explorer",
            onSelect: () => {
              revealArchive(collection.id).catch((e) => setError(errorMessage(e)));
            },
          },
        ],
      });
    },
    [],
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

  // ---- appearance ---------------------------------------------------------

  const viewModes = settings?.viewModes ?? DEFAULT_VIEW_MODES;
  // Sidebar kinds map onto the stored slots; collections and tags share one.
  const viewKey: keyof ViewModes =
    view.kind === "collection" || view.kind === "tag"
      ? "other"
      : view.kind === "image"
        ? "images"
        : view.kind;
  const viewMode: ViewMode = viewModes[viewKey] === "grid" ? "grid" : "list";
  const gridScale = settings?.gridScale ?? 1;
  const textScale = settings?.textScale ?? 100;
  const detailOpen = settings?.detailPaneOpen ?? true;
  const sortOrder = settings?.sortOrder ?? "updated-desc";

  // While the slider is moving the grid follows a local value, so resizing is
  // instant; the setting catches up once the gesture pauses.
  const [liveGridScale, setLiveGridScale] = useState<number | null>(null);
  const gridPercent = liveGridScale ?? Math.round(gridScale * 100);

  const changeGridScale = useCallback((percent: number) => setLiveGridScale(percent), []);

  useEffect(() => {
    if (liveGridScale === null) return;
    const timer = window.setTimeout(() => {
      void applyPatch({ gridScale: liveGridScale / 100 });
    }, 300);
    return () => window.clearTimeout(timer);
  }, [liveGridScale, applyPatch]);

  // Once the stored value matches the drag, hand control back to the settings.
  useEffect(() => {
    if (liveGridScale !== null && Math.round(gridScale * 100) === liveGridScale) {
      setLiveGridScale(null);
    }
  }, [gridScale, liveGridScale]);

  const toggleDetail = useCallback(() => {
    void applyPatch({ detailPaneOpen: !detailOpen });
  }, [applyPatch, detailOpen]);

  const changeViewMode = useCallback(
    (mode: ViewMode) => {
      void applyPatch({ viewModes: { ...viewModes, [viewKey]: mode } });
    },
    [applyPatch, viewModes, viewKey],
  );

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
          // While a query is being edited, Delete belongs to the text field.
          // Removing a clip from under the cursor would be a nasty surprise.
          if (inSearch && query.length > 0) return;
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

  // Failures from any layer surface in one banner; successes share the strip,
  // except on the settings page, which draws its own notice inside the panel.
  const banner = error ?? settingsError ?? historyError;
  const info = !banner && route === "history" ? notice : null;

  // Notices are confirmations, not state the user has to dismiss.
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 3_000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  // ---- interface scale ----------------------------------------------------
  //
  // Set as a custom property rather than an inline size on the app container:
  // `#root` owns the zoom (see index.css), so nothing else has to compensate for
  // it and the layout keeps filling the window.
  useEffect(() => {
    document.documentElement.style.setProperty("--ui-scale", String(textScale / 100));
  }, [textScale]);

  // The palette is stored with the rest of the settings, so it is applied here
  // rather than once at boot.
  useEffect(() => {
    if (settings?.theme) applyTheme(settings.theme);
  }, [settings?.theme]);

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

      {(banner || info) && (
        <div
          role={banner ? "alert" : "status"}
          className={`flex shrink-0 items-center gap-2 border-b px-3 py-1.5 text-[11px] ${
            banner
              ? "border-danger/30 bg-danger/10 text-danger"
              : "border-ok/30 bg-ok/10 text-ok"
          }`}
        >
          <span className="min-w-0 flex-1">{banner ?? info}</span>
          <button
            type="button"
            onClick={() => {
              setError(null);
              setNotice(null);
              clearError();
            }}
            className="shrink-0 underline underline-offset-2"
          >
            Dismiss
          </button>
        </div>
      )}

      {updater.state.kind === "available" && (
        <div
          role="status"
          className="flex shrink-0 items-center gap-2 border-b border-accent/30 bg-accent/10 px-3 py-1.5 text-[11px] text-accent-hi"
        >
          <span className="min-w-0 flex-1">CopyPaste {updater.state.version} is available.</span>
          <button
            type="button"
            onClick={() => void updater.install()}
            className="shrink-0 rounded bg-accent/25 px-2 py-0.5 font-medium text-accent-hi transition-colors hover:bg-accent/35"
          >
            Install
          </button>
          <button
            type="button"
            onClick={updater.dismiss}
            className="shrink-0 underline underline-offset-2"
          >
            Later
          </button>
        </div>
      )}

      {updater.state.kind === "downloading" && (
        <div
          role="status"
          className="flex shrink-0 items-center gap-2 border-b border-accent/30 bg-accent/10 px-3 py-1.5 text-[11px] text-accent-hi"
        >
          <span className="min-w-0 flex-1">
            Downloading CopyPaste {updater.state.version}
            {updater.state.percent !== null ? `… ${updater.state.percent}%` : "…"}
          </span>
        </div>
      )}

      {updater.state.kind === "error" && (
        <div
          role="alert"
          className="flex shrink-0 items-center gap-2 border-b border-danger/30 bg-danger/10 px-3 py-1.5 text-[11px] text-danger"
        >
          <span className="min-w-0 flex-1">{updater.state.message}</span>
          <button
            type="button"
            onClick={updater.dismiss}
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
          appVersion={appVersion}
          updateState={updater.state}
          onCheckForUpdates={() => void updater.checkNow()}
          onPatch={applyPatch}
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
            onArchiveCollection={(collection, archived) =>
              void archiveCollection(collection, archived)
            }
            onContextMenuCollection={openArchiveMenu}
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
              viewMode={viewMode}
              onViewModeChange={changeViewMode}
              onZoomChange={changeGridScale}
              zoomPercent={gridPercent}
              sortOrder={sortOrder}
              onSortChange={(order) => void applyPatch({ sortOrder: order })}
            />

            <div className="flex min-h-0 flex-1">
              {viewMode === "grid" ? (
                <ClipGrid
                  items={items}
                  selectedId={selectedId}
                  loading={loading}
                  emptyMessage={emptyMessage}
                  now={now}
                  scale={gridPercent / 100}
                  onSelect={setSelectedId}
                  onActivate={(clip) => void activate(clip)}
                  onCopy={(clip) => void copySilently(clip)}
                  onCopyText={(clip) => void copyTextFromImage(clip)}
                  onContextMenu={openClipMenu}
                  onTogglePin={(clip) => void togglePin(clip)}
                  onDelete={(clip) => void removeClip(clip)}
                />
              ) : (
                <ClipList
                  items={items}
                  selectedId={selectedId}
                  loading={loading}
                  emptyMessage={emptyMessage}
                  now={now}
                  onSelect={setSelectedId}
                  onActivate={(clip) => void activate(clip)}
                  onCopy={(clip) => void copySilently(clip)}
                  onContextMenu={openClipMenu}
                  onTogglePin={(clip) => void togglePin(clip)}
                  onDelete={(clip) => void removeClip(clip)}
                />
              )}

              {/* Cards are self-describing, so the grid gets the detail pane's width
                  instead. Switch to list view to edit tags and collections. */}
              {viewMode === "list" && (
                <ClipDetailPane
                  clip={selectedClip}
                  detail={detail}
                  loadingDetail={loadingDetail}
                  collections={collections}
                  now={now}
                  open={detailOpen}
                  onToggle={toggleDetail}
                  onCopy={(clip) => void activate(clip)}
                  onCopyText={(clip) => void copyTextFromImage(clip)}
                  onTogglePin={(clip) => void togglePin(clip)}
                  onDelete={(clip) => void removeClip(clip)}
                  onTagsCommit={(clip, next) => void commitTags(clip, next)}
                  onCollectionsChange={(clip, ids) => void commitCollections(clip, ids)}
                />
              )}
            </div>
          </main>
        </div>
      )}

      <StatusBar stats={stats} settings={settings} shown={items.length} total={total} />

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          scale={textScale / 100}
          items={menu.items}
          onClose={closeMenu}
        />
      )}
    </div>
  );
}
