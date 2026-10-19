import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { DragEvent as ReactDragEvent } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { ClipDetailPane } from "./components/ClipDetailPane";
import { ClipGrid } from "./components/ClipGrid";
import { ClipList, scrollRowIntoView } from "./components/ClipList";
import { ContextMenu, type ContextMenuItem } from "./components/ContextMenu";
import { EmojiBrowser } from "./components/EmojiBrowser";
import { GifBrowser } from "./components/GifBrowser";
import { PromptDialog } from "./components/PromptDialog";
import { WhatsNewDialog } from "./components/WhatsNewDialog";
import { SearchBar, type ViewMode } from "./components/SearchBar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { IconClose } from "./components/icons";
import { useDebounced, useHistory, useNow } from "./hooks/useHistory";
import { useSettings } from "./hooks/useSettings";
import { useUpdater } from "./hooks/useUpdater";
import { fileUri } from "./lib/format";
import { releaseUrl } from "./lib/links";
import type { EmojiEntry } from "./lib/emoji";
import {
  addTagsMany,
  addToCollection,
  captureState,
  clearHistory,
  copyClip,
  copyOcrText,
  copyText,
  createCollection,
  deleteClip,
  deleteClips,
  deleteCollection,
  deleteTag,
  errorMessage,
  getClip,
  getStats,
  hideWindow,
  listCollections,
  listTags,
  openPath,
  openUrl,
  pauseCapture,
  pickScreenColor,
  pruneHistory,
  renameTag,
  resumeCapture,
  revealArchive,
  revealClip,
  revealDataDir,
  revealPath,
  saveClip,
  saveClips,
  setClipArchive,
  setClipCollections,
  setClipNote,
  setClipTags,
  setClipText,
  setCollectionArchived,
  setPinned,
  setPinnedMany,
  setTagColor,
  type PauseState,
} from "./lib/ipc";
import type { ClipAction } from "./lib/actions";
import { parseSearch, removeToken } from "./lib/search";
import { applyTheme } from "./lib/theme";
import {
  DEFAULT_VIEW_MODES,
  viewFilter,
  viewLabel,
  viewQuery,
  type ClipDetail,
  type ClipSummary,
  type Collection,
  type GifFavorite,
  type SavedSearch,
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
const CAPTURE_PAUSED = "capture:paused";
const COLOR_PICKED = "ui:color-picked";

/** Payload of `COLOR_PICKED`; `error` is set when the pick was cancelled. */
interface ColorPickPayload {
  hex: string | null;
  id: number | null;
  error?: string | null;
}

/** The one-line questions the UI asks through `PromptDialog`. */
type Prompt =
  | { kind: "rename-tag"; tag: Tag }
  | { kind: "tag-color"; tag: Tag }
  | { kind: "bulk-tag" }
  | { kind: "bulk-collect" }
  | { kind: "save-search" }
  | { kind: "rename-search"; search: SavedSearch };

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
  /** Clips picked with Ctrl/Shift click, for the bulk bar. */
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  /** Include clips filed in archive collections in the current search. */
  const [includeArchived, setIncludeArchived] = useState(false);
  const [pause, setPause] = useState<PauseState | null>(null);
  const [prompt, setPrompt] = useState<Prompt | null>(null);

  const searchRef = useRef<HTMLInputElement | null>(null);
  const { settings, error: settingsError, patch, clearError } = useSettings();
  const now = useNow();
  const [appVersion, setAppVersion] = useState<string | null>(null);
  /** Version that was running before this launch, when it changed. */
  const [updatedFrom, setUpdatedFrom] = useState<string | null>(null);
  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => undefined);
  }, []);

  // An update restarts the app, so a version change since the last launch is
  // exactly the moment to offer the release notes.
  useEffect(() => {
    if (!appVersion) return;
    try {
      const seen = localStorage.getItem("copypaste.lastVersion");
      if (seen && seen !== appVersion) setUpdatedFrom(seen);
      localStorage.setItem("copypaste.lastVersion", appVersion);
    } catch {
      // Storage being unavailable only costs the one-time prompt.
    }
  }, [appVersion]);

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
  //
  // Operators are parsed from the live query so the chips appear as soon as one is
  // typed, while the store query itself waits for the debounce.
  const parsed = useMemo(() => parseSearch(query, collections), [query, collections]);
  const searchText = useDebounced(parsed.text, 70);
  const filter = useMemo(
    () => ({
      ...viewFilter(view),
      ...parsed.filter,
      includeArchived: includeArchived || parsed.filter.includeArchived === true,
    }),
    [view, parsed, includeArchived],
  );
  const { items, total, loading, error: historyError } = useHistory(filter, searchText);

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

  // A screen sample finishes after the window has been restored, so route the
  // result into the Colors view and keep the sampled clip selected.
  useEffect(() => {
    const pending = listen<ColorPickPayload>(COLOR_PICKED, (event) => {
      const { hex, id, error } = event.payload;
      if (error || !hex) {
        setError(error ?? "no colour was picked");
        return;
      }
      setRoute("history");
      setView({ kind: "colors" });
      setQuery("");
      if (typeof id === "number") setSelectedId(id);
      setNotice(`Copied ${hex} and added it to Colors.`);
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

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

  const commitNote = useCallback(
    async (clip: ClipSummary, note: string | null) => {
      try {
        await setClipNote(clip.id, note);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const commitText = useCallback(
    async (clip: ClipSummary, text: string) => {
      try {
        await setClipText(clip.id, text);
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

  /** Files a clip into one archive (or none), which is a single slot. */
  const commitArchive = useCallback(
    async (clip: ClipSummary, archiveId: number | null) => {
      try {
        await setClipArchive(clip.id, archiveId);
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

  // ---- tag management -----------------------------------------------------

  const renameExistingTag = useCallback(
    async (tag: Tag, name: string) => {
      try {
        await renameTag(tag.id, name);
        if (view.kind === "tag" && view.name === tag.name) {
          setView({ kind: "tag", name });
        }
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, view],
  );

  const recolorTag = useCallback(
    async (tag: Tag, color: string | null) => {
      try {
        await setTagColor(tag.id, color);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const removeTag = useCallback(
    async (tag: Tag) => {
      try {
        await deleteTag(tag.id);
        if (view.kind === "tag" && view.name === tag.name) setView({ kind: "all" });
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, view],
  );

  // ---- saved searches -----------------------------------------------------

  const saveSearch = useCallback(
    async (name: string) => {
      const existing = settings?.savedSearches ?? [];
      const next: SavedSearch[] = [
        ...existing.filter((search) => search.name.toLowerCase() !== name.toLowerCase()),
        { name, query: query.trim() },
      ];
      await applyPatch({ savedSearches: next });
      setNotice(`Saved “${name}” to the sidebar.`);
    },
    [applyPatch, query, settings?.savedSearches],
  );

  const renameSearch = useCallback(
    async (search: SavedSearch, name: string) => {
      const next = (settings?.savedSearches ?? []).map((entry) =>
        entry.name === search.name ? { ...entry, name } : entry,
      );
      await applyPatch({ savedSearches: next });
      if (view.kind === "search" && view.name === search.name) {
        setView({ kind: "search", name, query: search.query });
      }
    },
    [applyPatch, settings?.savedSearches, view],
  );

  const removeSearch = useCallback(
    async (search: SavedSearch) => {
      const next = (settings?.savedSearches ?? []).filter((entry) => entry.name !== search.name);
      await applyPatch({ savedSearches: next });
      if (view.kind === "search" && view.name === search.name) setView({ kind: "all" });
    },
    [applyPatch, settings?.savedSearches, view],
  );

  // ---- payload actions ----------------------------------------------------

  const saveAs = useCallback(async (clip: ClipSummary) => {
    try {
      const chosen = await openDialog({
        directory: true,
        multiple: false,
        title: "Save this clip to a folder",
      });
      if (typeof chosen !== "string") return;
      const written = await saveClip(clip.id, chosen);
      setNotice(
        written.length === 1
          ? `Saved ${written[0].split(/[\\/]/).pop()}.`
          : `Saved ${written.length} files to ${chosen}.`,
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  const openLink = useCallback((url: string) => {
    openUrl(url).catch((e) => setError(errorMessage(e)));
  }, []);

  const openFile = useCallback((path: string) => {
    openPath(path).catch((e) => setError(errorMessage(e)));
  }, []);

  const showPath = useCallback((path: string) => {
    revealPath(path).catch((e) => setError(errorMessage(e)));
  }, []);

  /** Runs a clip type's own action from the list or grid. */
  const runClipAction = useCallback(
    async (action: ClipAction) => {
      let target = action.target;
      try {
        // The list preview truncates long payloads; the detail has the real one.
        const detail = await getClip(action.clipId);
        target = detail.text?.trim() || target;
      } catch (e) {
        setError(errorMessage(e));
        return;
      }
      if (action.kind === "open-url") openLink(target);
      else showPath(target);
    },
    [openLink, showPath],
  );

  // ---- utilities ----------------------------------------------------------

  const showError = useCallback((text: string) => setError(text), []);
  const showNotice = useCallback((text: string) => setNotice(text), []);

  /** Hides the window and samples the next clicked pixel. */
  const pickColor = useCallback(() => {
    pickScreenColor().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Copies a shelf clip by id; the GIF browser only carries the id. */
  const copyClipById = useCallback(async (id: number) => {
    try {
      await copyClip(id, false, false);
      setNotice("Copied the clip.");
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  const changeGifFavorites = useCallback(
    (favorites: GifFavorite[]) => {
      void applyPatch({ gifFavorites: favorites });
    },
    [applyPatch],
  );

  const commitGifScale = useCallback(
    (next: number) => {
      void applyPatch({ gifScale: next });
    },
    [applyPatch],
  );

  /** Adds or removes an image clip on the GIF shelf. */
  const toggleClipGif = useCallback(
    (clip: ClipSummary) => {
      const current = settings?.gifFavorites ?? [];
      const id = `clip:${clip.id}`;
      const next = current.some((favorite) => favorite.id === id)
        ? current.filter((favorite) => favorite.id !== id)
        : [
            {
              id,
              title: (clip.preview || "Image clip").slice(0, 120),
              url: "",
              previewUrl: "",
              clipId: clip.id,
            },
            ...current,
          ];
      void applyPatch({ gifFavorites: next.slice(0, 100) });
    },
    [applyPatch, settings?.gifFavorites],
  );

  // ---- emoji --------------------------------------------------------------

  const copyEmoji = useCallback((entry: EmojiEntry) => {
    copyText(entry.char)
      .then(() => setNotice(`Copied ${entry.char} ${entry.name}.`))
      .catch((e) => setError(errorMessage(e)));
  }, []);

  /** Puts an error or notice on the clipboard, so it can be pasted into a report. */
  const copyMessage = useCallback((text: string) => {
    copyText(text)
      .then(() => setNotice("Copied the message."))
      .catch((e) => setError(errorMessage(e)));
  }, []);

  const toggleEmojiFavorite = useCallback(
    (char: string, remove: boolean) => {
      const current = settings?.emojiFavorites ?? [];
      const next = remove
        ? current.filter((existing) => existing !== char)
        : [...current.filter((existing) => existing !== char), char];
      void applyPatch({ emojiFavorites: next });
    },
    [applyPatch, settings?.emojiFavorites],
  );

  // ---- context menus ------------------------------------------------------

  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    items: ContextMenuItem[];
  } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const openTagMenu = useCallback(
    (tag: Tag, event: ReactMouseEvent) => {
      event.preventDefault();
      setMenu({
        x: event.clientX,
        y: event.clientY,
        items: [
          { label: "Rename…", onSelect: () => setPrompt({ kind: "rename-tag", tag }) },
          { label: "Color…", onSelect: () => setPrompt({ kind: "tag-color", tag }) },
          { label: "Delete", onSelect: () => void removeTag(tag) },
        ],
      });
    },
    [removeTag],
  );

  const openSearchMenu = useCallback(
    (search: SavedSearch, event: ReactMouseEvent) => {
      event.preventDefault();
      setMenu({
        x: event.clientX,
        y: event.clientY,
        items: [
          { label: "Rename…", onSelect: () => setPrompt({ kind: "rename-search", search }) },
          { label: "Delete", onSelect: () => void removeSearch(search) },
        ],
      });
    },
    [removeSearch],
  );

  const openEmojiMenu = useCallback(
    (entry: EmojiEntry, event: ReactMouseEvent, favourite: boolean) => {
      event.preventDefault();
      setMenu({
        x: event.clientX,
        y: event.clientY,
        items: [
          { label: "Copy", onSelect: () => copyEmoji(entry) },
          favourite
            ? {
                label: "Remove from favourites",
                onSelect: () => toggleEmojiFavorite(entry.char, true),
              }
            : { label: "Add to favourites", onSelect: () => toggleEmojiFavorite(entry.char, false) },
        ],
      });
    },
    [copyEmoji, toggleEmojiFavorite],
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

  // ---- private mode -------------------------------------------------------

  useEffect(() => {
    captureState()
      .then(setPause)
      .catch(() => undefined);
    const pending = listen<PauseState>(CAPTURE_PAUSED, (event) => setPause(event.payload));
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  const applyPauseState = useCallback((next: PauseState) => {
    setPause(next);
  }, []);

  const pauseFor = useCallback(
    (minutes?: number, untilRestart?: boolean) => {
      pauseCapture(minutes, untilRestart)
        .then((next) => {
          applyPauseState(next);
          setNotice(
            untilRestart
              ? "Capture paused until CopyPaste restarts."
              : `Capture paused for ${minutes ?? 15} minute(s).`,
          );
        })
        .catch((e) => setError(errorMessage(e)));
    },
    [applyPauseState],
  );

  const resumeCapturing = useCallback(() => {
    resumeCapture()
      .then((next) => {
        applyPauseState(next);
        setNotice("Capture resumed.");
      })
      .catch((e) => setError(errorMessage(e)));
  }, [applyPauseState]);

  // ---- selection ----------------------------------------------------------

  const selectClip = useCallback(
    (id: number, modifiers: { ctrl: boolean; shift: boolean }) => {
      if (modifiers.shift && selectedId !== null) {
        const from = items.findIndex((item) => item.id === selectedId);
        const to = items.findIndex((item) => item.id === id);
        if (from !== -1 && to !== -1) {
          const [start, end] = from < to ? [from, to] : [to, from];
          setSelectedIds(items.slice(start, end + 1).map((item) => item.id));
          setSelectedId(id);
          return;
        }
      }
      if (modifiers.ctrl) {
        setSelectedIds((current) =>
          current.includes(id) ? current.filter((existing) => existing !== id) : [...current, id],
        );
        setSelectedId(id);
        return;
      }
      setSelectedIds([]);
      setSelectedId(id);
    },
    [items, selectedId],
  );

  const clearSelection = useCallback(() => setSelectedIds([]), []);

  const picked = useMemo(
    () => (selectedIds.length > 1 ? selectedIds : []),
    [selectedIds],
  );

  // ---- bulk actions -------------------------------------------------------

  const bulkPin = useCallback(
    async (pinned: boolean) => {
      try {
        await setPinnedMany(picked, pinned);
        clearSelection();
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, clearSelection, picked],
  );

  const bulkDelete = useCallback(async () => {
    try {
      const removed = await deleteClips(picked);
      clearSelection();
      bump();
      setNotice(`Removed ${removed} clip${removed === 1 ? "" : "s"}.`);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [bump, clearSelection, picked]);

  const bulkTag = useCallback(
    async (name: string) => {
      try {
        await addTagsMany(picked, [name]);
        clearSelection();
        bump();
        setNotice(`Tagged ${picked.length} clips with #${name}.`);
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, clearSelection, picked],
  );

  const bulkCollect = useCallback(
    async (collectionId: number) => {
      try {
        await addToCollection(picked, collectionId);
        clearSelection();
        bump();
        setNotice(`Filed ${picked.length} clips.`);
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump, clearSelection, picked],
  );

  /**
   * Files the selection into a collection chosen by name, creating it when the
   * name is new. The bulk bar used to send a NaN collection id for a typed name,
   * which the backend correctly rejected with "invalid type: null".
   */
  const bulkCollectByName = useCallback(
    async (value: string) => {
      const typed = value.trim();
      if (!typed) return;
      try {
        const match = collections.find(
          (collection) => collection.name.toLowerCase() === typed.toLowerCase(),
        );
        let collectionId = match?.id;
        if (collectionId === undefined) {
          const created = await createCollection(typed);
          collectionId = created.id;
        }
        await bulkCollect(collectionId);
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bulkCollect, collections],
  );

  /** Exports the selection into a folder the user picks. */
  const bulkExport = useCallback(async () => {
    try {
      const chosen = await openDialog({
        directory: true,
        multiple: false,
        title: `Export ${picked.length} clips to a folder`,
      });
      if (typeof chosen !== "string") return;
      const written = await saveClips(picked, chosen);
      clearSelection();
      setNotice(`Exported ${written.length} file${written.length === 1 ? "" : "s"}.`);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [clearSelection, picked]);

  const openClipMenu = useCallback(
    (clip: ClipSummary, event: ReactMouseEvent) => {
      event.preventDefault();
      const items: ContextMenuItem[] = [];
      // Right-clicking inside the selection acts on all of it; right-clicking
      // elsewhere falls back to the single clip under the cursor.
      const selection = picked.includes(clip.id) ? picked : [];

      if (selection.length > 1) {
        const count = selection.length;
        items.push({
          label: `Pin ${count} clips`,
          onSelect: () => void bulkPin(true),
        });
        items.push({
          label: `Unpin ${count} clips`,
          onSelect: () => void bulkPin(false),
        });
        items.push({
          label: `Tag ${count} clips…`,
          onSelect: () => setPrompt({ kind: "bulk-tag" }),
        });
        items.push({
          label: `Collect ${count} clips…`,
          onSelect: () => setPrompt({ kind: "bulk-collect" }),
        });
        items.push({
          label: `Save ${count} clips to…`,
          onSelect: () => void bulkExport(),
        });
        items.push({
          label: `Delete ${count} clips`,
          onSelect: () => void bulkDelete(),
        });
      }

      const single = selection.length > 1;
      const group = (item: ContextMenuItem): ContextMenuItem =>
        single ? { ...item, separatorBefore: true } : item;

      if (clip.contentKind === "url") {
        const url = clip.preview.trim();
        items.push(group({ label: "Open link", onSelect: () => openLink(url) }));
      }
      if (clip.contentKind === "path") {
        const path = clip.preview.trim();
        items.push(group({ label: "Open", onSelect: () => openFile(path) }));
        items.push({ label: "Show in Explorer", onSelect: () => showPath(path) });
      }
      if (clip.kind !== "text") {
        items.push(
          group({
            label: clip.kind === "image" ? "Show image in Explorer" : "Show in Explorer",
            onSelect: () => {
              revealClip(clip.id).catch((e) => setError(errorMessage(e)));
            },
          }),
        );
      }
      if (clip.kind === "image" && clip.hasOcr) {
        items.push({
          label: "Copy text from image",
          onSelect: () => void copyTextFromImage(clip),
        });
      }
      items.push(group({ label: "Save to folder…", onSelect: () => void saveAs(clip) }));
      items.push({ label: "Copy", onSelect: () => void copySilently(clip) });
      items.push({
        label: clip.pinned ? "Unpin" : "Pin",
        onSelect: () => void togglePin(clip),
      });
      setMenu({ x: event.clientX, y: event.clientY, items });
    },
    [
      bulkDelete,
      bulkExport,
      bulkPin,
      copySilently,
      copyTextFromImage,
      openFile,
      openLink,
      picked,
      saveAs,
      showPath,
      togglePin,
    ],
  );


  // ---- dragging clips out -------------------------------------------------

  // The payload of a clip lives in the encrypted store, and a drag has to hand its
  // data over synchronously, so hovering a row pre-loads it into this cache.
  const dragCache = useRef(new Map<number, ClipDetail>());
  const prefetch = useCallback((clip: ClipSummary) => {
    if (dragCache.current.has(clip.id)) return;
    getClip(clip.id)
      .then((loaded) => dragCache.current.set(clip.id, loaded))
      .catch(() => undefined);
  }, []);

  const startDrag = useCallback((clip: ClipSummary, event: ReactDragEvent) => {
    const loaded = dragCache.current.get(clip.id);
    event.dataTransfer.effectAllowed = "copy";
    if (clip.kind === "text") {
      event.dataTransfer.setData("text/plain", loaded?.text ?? clip.preview);
    } else if (clip.kind === "image" && clip.imagePath) {
      event.dataTransfer.setData("text/uri-list", fileUri(clip.imagePath));
    } else if (clip.kind === "files" && loaded) {
      event.dataTransfer.setData("text/uri-list", loaded.filePaths.map(fileUri).join("\r\n"));
    }
  }, []);


  // ---- appearance ---------------------------------------------------------

  const viewModes = settings?.viewModes ?? DEFAULT_VIEW_MODES;
  // Sidebar kinds map onto the stored slots; collections, tags and saved searches
  // share one.
  const viewKey: keyof ViewModes =
    view.kind === "collection" || view.kind === "tag" || view.kind === "search"
      ? "other"
      : view.kind === "image"
        ? "images"
        : view.kind === "emoji"
          ? "all"
          : view.kind === "gifs"
            ? "other"
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
        if (selectedIds.length > 0) {
          setSelectedIds([]);
          return;
        }
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

      // Quick paste: Ctrl+1 … Ctrl+9 activate the clip with that badge.
      if ((event.ctrlKey || event.metaKey) && /^[1-9]$/.test(event.key)) {
        const clipboardClip = items[Number(event.key) - 1];
        if (!clipboardClip) return;
        event.preventDefault();
        setSelectedId(clipboardClip.id);
        void activate(clipboardClip);
        return;
      }

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
        // Inside the search box, Ctrl+A selects the text, as it should everywhere.
        if (inSearch || items.length === 0) return;
        event.preventDefault();
        setSelectedIds(items.map((item) => item.id));
        return;
      }

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
          if (picked.length > 1) {
            event.preventDefault();
            void bulkDelete();
            return;
          }
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
  }, [
    items,
    selectedId,
    selectedClip,
    selectedIds,
    picked,
    query,
    route,
    activate,
    bulkDelete,
    removeClip,
    togglePin,
  ]);

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
        onPickColor={pickColor}
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
          <span data-selectable className="min-w-0 flex-1">
            {banner ?? info}
          </span>
          {banner && (
            <button
              type="button"
              onClick={() => copyMessage(banner)}
              className="shrink-0 underline underline-offset-2"
            >
              Copy
            </button>
          )}
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
          onOpenChangelog={(version) => openLink(releaseUrl(version))}
          onPatch={applyPatch}
          onDismissError={() => setError(null)}
          onRevealDataDir={openDataDir}
          onClearHistory={(keepPinned) => void runClear(keepPinned)}
          onPrune={() => void runPrune()}
          pausedUntil={pause?.paused && !pause.indefinite ? pause.until : null}
          pausedIndefinitely={pause?.indefinite ?? false}
          onPause={pauseFor}
          onResume={resumeCapturing}
          now={now}
        />
      ) : (
        <div className="flex min-h-0 flex-1">
          <Sidebar
            view={view}
            onSelect={(next) => {
              setView(next);
              setQuery(viewQuery(next));
              setSelectedIds([]);
            }}
            stats={stats}
            collections={collections}
            tags={tags}
            savedSearches={settings?.savedSearches ?? []}
            onCreateCollection={(name) => void addCollection(name)}
            onDeleteCollection={(collection) => void removeCollection(collection)}
            onArchiveCollection={(collection, archived) =>
              void archiveCollection(collection, archived)
            }
            onContextMenuCollection={openArchiveMenu}
            onContextMenuTag={openTagMenu}
            onContextMenuSearch={openSearchMenu}
          />

          <main className="flex min-w-0 flex-1 flex-col">
            {view.kind === "gifs" ? (
              <GifBrowser
                favorites={settings?.gifFavorites ?? []}
                hasApiKey={Boolean(settings?.giphyApiKey.trim())}
                scale={settings?.gifScale ?? 1}
                onScaleCommit={commitGifScale}
                onFavoritesChange={changeGifFavorites}
                onCopyClip={(id) => void copyClipById(id)}
                onNotice={showNotice}
                onError={showError}
              />
            ) : view.kind === "emoji" ? (
              <EmojiBrowser
                favorites={settings?.emojiFavorites ?? []}
                onCopy={copyEmoji}
                onContextMenu={openEmojiMenu}
              />
            ) : (
              <>
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
              chips={parsed.chips}
              onRemoveChip={(token) => setQuery((current) => removeToken(current, token))}
              includeArchived={includeArchived}
              onToggleIncludeArchived={() => setIncludeArchived((current) => !current)}
              onSaveSearch={() => setPrompt({ kind: "save-search" })}
              canSaveSearch={query.trim().length > 0}
            />

            {picked.length > 1 && (
              <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-accent/30 bg-accent/10 px-3 py-1.5 text-[11px] text-accent-hi">
                <span className="shrink-0 whitespace-nowrap font-medium">
                  {picked.length} selected
                </span>
                <button
                  type="button"
                  onClick={() => void bulkPin(true)}
                  className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  Pin
                </button>
                <button
                  type="button"
                  onClick={() => void bulkPin(false)}
                  className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  Unpin
                </button>
                <button
                  type="button"
                  onClick={() => setPrompt({ kind: "bulk-tag" })}
                  className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  Tag…
                </button>
                <button
                  type="button"
                  onClick={() => setPrompt({ kind: "bulk-collect" })}
                  className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  Collect…
                </button>
                <button
                  type="button"
                  onClick={() => void bulkExport()}
                  className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  Save to…
                </button>
                <button
                  type="button"
                  onClick={() => void bulkDelete()}
                  className="shrink-0 whitespace-nowrap rounded border border-danger/50 bg-danger/15 px-1.5 py-0.5 text-[10px] text-danger transition-colors hover:bg-danger/25"
                >
                  Delete
                </button>
                <span className="flex-1" />
                <button
                  type="button"
                  onClick={clearSelection}
                  title="Clear the selection"
                  aria-label="Clear the selection"
                  className="shrink-0 rounded p-0.5 text-accent-hi transition-colors hover:bg-accent/20"
                >
                  <IconClose className="size-3" />
                </button>
              </div>
            )}

            <div className="flex min-h-0 flex-1">
              {viewMode === "grid" ? (
                <ClipGrid
                  items={items}
                  selectedId={selectedId}
                  selectedIds={selectedIds}
                  loading={loading}
                  emptyMessage={emptyMessage}
                  now={now}
                  scale={gridPercent / 100}
                  onSelect={selectClip}
                  onActivate={(clip) => void activate(clip)}
                  onCopy={(clip) => void copySilently(clip)}
                  onCopyText={(clip) => void copyTextFromImage(clip)}
                  onContextMenu={openClipMenu}
                  onTogglePin={(clip) => void togglePin(clip)}
                  onDelete={(clip) => void removeClip(clip)}
                  onRunAction={runClipAction}
                  onPrefetch={prefetch}
                  onDragStart={startDrag}
                />
              ) : (
                <ClipList
                  items={items}
                  selectedId={selectedId}
                  selectedIds={selectedIds}
                  loading={loading}
                  emptyMessage={emptyMessage}
                  now={now}
                  onSelect={selectClip}
                  onActivate={(clip) => void activate(clip)}
                  onCopy={(clip) => void copySilently(clip)}
                  onContextMenu={openClipMenu}
                  onTogglePin={(clip) => void togglePin(clip)}
                  onDelete={(clip) => void removeClip(clip)}
                  onRunAction={runClipAction}
                  onPrefetch={prefetch}
                  onDragStart={startDrag}
                />
              )}

              {/* The pane is useful in both layouts: card view still benefits from
                  tags, collections and the full preview. */}
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
                onSaveAs={(clip) => void saveAs(clip)}
                onOpenUrl={openLink}
                onOpenPath={openFile}
                onRevealPath={showPath}
                archives={collections.filter((collection) => collection.archived)}
                width={settings?.detailPaneWidth ?? 286}
                onWidthChange={() => undefined}
                onWidthCommit={(next) => void applyPatch({ detailPaneWidth: Math.round(next) })}
                onArchiveChange={(clip, archiveId) => void commitArchive(clip, archiveId)}
                onToggleGif={toggleClipGif}
                gifFavorite={(settings?.gifFavorites ?? []).some(
                  (favorite) => favorite.id === `clip:${selectedClip?.id}`,
                )}
                onNoteCommit={(clip, note) => void commitNote(clip, note)}
                onTextCommit={(clip, text) => void commitText(clip, text)}
              />
            </div>
              </>
            )}
          </main>
        </div>
      )}

      <StatusBar
        stats={stats}
        settings={settings}
        shown={items.length}
        total={total}
        picked={picked.length}
        pausedUntil={pause?.paused && !pause.indefinite ? pause.until : null}
        pausedIndefinitely={pause?.indefinite ?? false}
        onResume={resumeCapturing}
        now={now}
      />

      {prompt && (
        <PromptDialog
          title={
            prompt.kind === "rename-tag"
              ? `Rename “${prompt.tag.name}”`
              : prompt.kind === "tag-color"
                ? `Color “${prompt.tag.name}”`
                : prompt.kind === "bulk-tag"
                  ? `Tag ${picked.length} clips`
                  : prompt.kind === "bulk-collect"
                    ? `File ${picked.length} clips`
                    : prompt.kind === "save-search"
                      ? "Save this search"
                      : `Rename “${prompt.search.name}”`
          }
          description={
            prompt.kind === "rename-tag"
              ? "Renaming onto a tag that already exists merges the two."
              : prompt.kind === "tag-color"
                ? "The color is used for the tag's dot in the sidebar."
                : prompt.kind === "bulk-tag"
                  ? "The tag is added to every selected clip; existing tags are kept."
                  : prompt.kind === "bulk-collect"
                    ? "Pick a collection, or type a new name to create one."
                    : prompt.kind === "save-search"
                      ? "Saved searches appear in the sidebar and re-run with one click."
                      : undefined
          }
          label={
            prompt.kind === "bulk-collect"
              ? "Collection"
              : prompt.kind === "tag-color"
                ? undefined
                : "Name"
          }
          initialValue={
            prompt.kind === "rename-tag"
              ? prompt.tag.name
              : prompt.kind === "tag-color"
                ? (prompt.tag.color ?? "#7c9cff")
                : prompt.kind === "rename-search"
                  ? prompt.search.name
                  : ""
          }
          kind={prompt.kind === "tag-color" ? "color" : "text"}
          confirmLabel={prompt.kind === "tag-color" ? "Apply" : "Save"}
          choices={
            prompt.kind === "tag-color"
              ? [{ label: "No color", value: "" }]
              : prompt.kind === "bulk-collect"
                ? collections
                    .filter((collection) => !collection.archived)
                    .map((collection) => ({
                      label: collection.name,
                      value: collection.name,
                    }))
                : undefined
          }
          onCancel={() => setPrompt(null)}
          onSubmit={(value) => {
            const current = prompt;
            setPrompt(null);
            if (current.kind === "rename-tag") void renameExistingTag(current.tag, value);
            else if (current.kind === "tag-color")
              void recolorTag(current.tag, value.trim() ? value : null);
            else if (current.kind === "bulk-tag") void bulkTag(value);
            else if (current.kind === "bulk-collect") void bulkCollectByName(value);            else if (current.kind === "save-search") void saveSearch(value);
            else if (current.kind === "rename-search") void renameSearch(current.search, value);
          }}
        />
      )}

      {updatedFrom !== null && appVersion && (
        <WhatsNewDialog
          version={appVersion}
          previous={updatedFrom}
          onOpenChangelog={() => {
            openLink(releaseUrl(appVersion));
            setUpdatedFrom(null);
          }}
          onDismiss={() => setUpdatedFrom(null)}
        />
      )}

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
