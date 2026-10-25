import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { DragEvent as ReactDragEvent } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { AddClipForm } from "./components/AddClipForm";
import { ClipDetailPane } from "./components/ClipDetailPane";
import { ClipGrid } from "./components/ClipGrid";
import { ClipList, scrollRowIntoView } from "./components/ClipList";
import { ClipPreviewOverlay } from "./components/ClipPreviewOverlay";
import { displayHex } from "./components/ColorSwatch";
import { ContextMenu, type ContextMenuItem } from "./components/ContextMenu";
import { VaultPrompt } from "./components/VaultPrompt";
import { VaultGateProvider } from "./lib/vaultGate";
import { windowKind } from "./lib/windowKind";
import { EmojiBrowser } from "./components/EmojiBrowser";
import { GifBrowser } from "./components/GifBrowser";
import { FirstRunWizard } from "./components/FirstRunWizard";
import { PromptDialog } from "./components/PromptDialog";
import { WhatsNewDialog } from "./components/WhatsNewDialog";
import { SearchBar, type ViewMode } from "./components/SearchBar";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TitleBar } from "./components/TitleBar";
import { IconArchive, IconClose, IconCopy, IconExternal, IconFolder, IconImage, IconList, IconMail, IconPin, IconPlus, IconSave, IconShield, IconTag, IconText, IconTrash } from "./components/icons";
import { useDebounced, useHistory, useNow } from "./hooks/useHistory";
import { useSettings } from "./hooks/useSettings";
import { useUpdater } from "./hooks/useUpdater";
import { colorSchemeText } from "./lib/colorScheme";
import { fileUri } from "./lib/format";
import { projectUrl, releaseUrl } from "./lib/links";
import type { EmojiEntry } from "./lib/emoji";
import {
  addSchemeColor,
  addTagsMany,
  changeVaultPassphrase,
  createVault,
  addToCollection,
  captureScreenRegion,
  captureScreenWindow,
  captureScrollingWindow,
  captureWholeScreen,
  captureState,
  clearHistory,
  copyClip,
  copyColor,
  copyOcrText,
  copyText,
  createCollection,
  deleteClip,
  deleteClips,
  deleteCollection,
  deleteTag,
  ensureColorScheme,
  errorMessage,
  exportMarkdown,
  getClip,
  getStats,
  hideWindow,
  importDroppedFiles,
  listCollections,
  listTags,
  openPath,
  openEmail,
  openAddClipWindow,
  openSettingsWindow,
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
  saveColorPalette,
  setClipArchive,
  setClipCollections,
  setClipSavedGif,
  setClipNote,
  lockVault,
  setClipSecret,
  unlockVault,
  vaultStatus as vaultStatusCall,
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
import { tagColorMap } from "./lib/tagColor";
import {
  DEFAULT_ARCHIVE_DISPLAY,
  DEFAULT_VIEW_MODES,
  viewFilter,
  viewLabel,
  viewQuery,
  type ClipDetail,
  type ClipSummary,
  type Collection,
  type GifFavorite,
  type MarkdownExport,
  type SavedSearch,
  type Settings,
  type SidebarView,
  type VaultStatus,
  type Stats,
  type CaptureMode,
  type Tag,
  type ViewModes,
  type ArchiveDisplay,
} from "./lib/types";

type Route = "history" | "settings" | "add-clip";

/** Sent by the backend when the tray or hotkey wants a particular pane shown. */
const ROUTE_EVENT = "ui:route";
const HISTORY_CHANGED = "history:changed";
const CAPTURE_PAUSED = "capture:paused";
const COLOR_PICKED = "ui:color-picked";
const SCREENSHOT_CAPTURED = "ui:screenshot-captured";

/** Payload of `COLOR_PICKED`; `error` is set when sampling failed. */
interface ColorPickPayload {
  hex: string | null;
  id: number | null;
  error?: string | null;
}

/** Payload of `SCREENSHOT_CAPTURED`; `error` is set when the capture failed. */
interface ScreenshotPayload {
  id: number | null;
  error?: string | null;
}

/** The one-line questions the UI asks through `PromptDialog`. */
type Prompt =
  | { kind: "rename-tag"; tag: Tag }
  | { kind: "tag-color"; tag: Tag }
  | { kind: "bulk-tag" }
  | { kind: "bulk-collect" }
  | { kind: "new-collection" }
  | { kind: "save-search" }
  | { kind: "rename-search"; search: SavedSearch };

/** Reports a Markdown export, folder included, in the notice strip. */
function markdownNotice(exported: MarkdownExport, dir: string): string {
  const notes = `${exported.written.length} note${exported.written.length === 1 ? "" : "s"}`;
  const skipped =
    exported.skipped > 0
      ? ` Skipped ${exported.skipped} clip${exported.skipped === 1 ? "" : "s"} that ${exported.skipped === 1 ? "is" : "are"} not text.`
      : "";
  const head =
    exported.written.length === 0
      ? `Nothing to export to ${dir}.`
      : `Exported ${notes} to ${dir}.`;
  return head + skipped;
}

export default function App() {
  // The settings and add-clip windows load the same bundle and are identified by
  // their window label, so each starts on its own page and closes with its own
  // chrome.
  const windowLabel = windowKind();
  const standaloneSettings = windowLabel === "settings";
  const [route, setRoute] = useState<Route>(
    windowLabel === "settings" ? "settings" : windowLabel === "add-clip" ? "add-clip" : "history",
  );
  const [view, setView] = useState<SidebarView>({ kind: "all" });
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  /** The clip shown in the Space preview, or null when it is closed. */
  const [previewId, setPreviewId] = useState<number | null>(null);
  const [detail, setDetail] = useState<ClipDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  /**
   * The colour scheme of the image on screen. The properties pane and the
   * preview's colour row show the same swatches, so it lives here once.
   */
  const [scheme, setScheme] = useState<{ clipId: number; colors: string[] } | null>(null);
  const [schemeBusy, setSchemeBusy] = useState(false);
  const [schemeError, setSchemeError] = useState<string | null>(null);
  /** The clip whose payload `detail` holds, so refreshes never blank the pane. */
  const detailClipId = useRef<number | null>(null);
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
  /**
   * The clips a "New collection…" prompt should file once it has a name.
   *
   * Kept beside the prompt rather than inside it, because the dialog is a plain
   * string field and this is the target list it was opened for.
   */
  const [newCollectionTargets, setNewCollectionTargets] = useState<number[]>([]);
  /** A Markdown export waiting on the "write a likely secret to disk" answer. */
  const [pendingMarkdown, setPendingMarkdown] = useState<{
    ids: number[];
    dir: string;
    hide: boolean;
    secrets: number;
  } | null>(null);
  /** Opened from Settings; the first run opens it automatically instead. */
  const [wizardOpen, setWizardOpen] = useState(false);
  /** Highlighted while files are dragged over the window. */
  const [dragging, setDragging] = useState(false);

  const searchRef = useRef<HTMLInputElement | null>(null);
  const { settings, error: settingsError, patch, refresh: refreshSettings, clearError } = useSettings();
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
  const { items, total, loading, error: historyError } = useHistory(
    filter,
    searchText,
    settings?.sortOrder ?? "updated-desc",
  );

  const bump = useCallback(() => setDataVersion((version) => version + 1), []);

  /** Files dropped on the window become history entries. */
  useEffect(() => {    let unlisten: (() => void) | undefined;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(true);
        } else if (payload.type === "leave") {
          setDragging(false);
        } else if (payload.type === "drop") {
          setDragging(false);
          if (payload.paths.length === 0) return;
          importDroppedFiles(payload.paths)
            .then((count) => {
              bump();
              setNotice(
                count === 0
                  ? "Those items are already in history."
                  : `Added ${count} dropped item${count === 1 ? "" : "s"} to history.`,
              );
            })
            .catch((e) => setError(errorMessage(e)));
        }
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [bump]);

  /** Archives can be told to keep their clips closed: no preview text and no
   * note marker, configured separately for rows and cards. */
  const archiveDisplay: ArchiveDisplay = useMemo(() => {
    if (view.kind !== "collection") {
      return { archiveId: 0, ...DEFAULT_ARCHIVE_DISPLAY };
    }
    const stored = settings?.archiveDisplay.find((entry) => entry.archiveId === view.id);
    return stored ?? { archiveId: view.id, ...DEFAULT_ARCHIVE_DISPLAY };
  }, [view, settings?.archiveDisplay]);

  const toggleSidebarSection = useCallback(
    (name: string) => {
      const current = settings?.sidebarCollapsed ?? [];
      const next = current.includes(name)
        ? current.filter((entry) => entry !== name)
        : [...current, name];
      void applyPatch({ sidebarCollapsed: next });
    },
    [settings?.sidebarCollapsed, applyPatch],
  );

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
      detailClipId.current = null;
      setDetail(null);
      return;
    }
    // Only a clip the pane has never shown needs the loading state. Filing a
    // clip into a collection bumps `dataVersion`, and blanking the payload for
    // that refresh is what made the properties pane flicker.
    const known = detailClipId.current === selectedId;
    let cancelled = false;
    if (!known) setLoadingDetail(true);
    getClip(selectedId)
      .then((loaded) => {
        if (cancelled) return;
        detailClipId.current = loaded.id;
        setDetail(loaded);
      })
      .catch(() => {
        if (cancelled) return;
        detailClipId.current = null;
        setDetail(null);
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
  // Tag colours are stored on the tag, not on the clip, so rows and cards look
  // them up by name.
  const tagColors = useMemo(() => tagColorMap(tags), [tags]);
  const previewClip = useMemo(
    () => (previewId === null ? null : (items.find((item) => item.id === previewId) ?? null)),
    [items, previewId],
  );

  // A clip that is filtered out or deleted, or leaving the history route,
  // dismounts the preview with it.
  useEffect(() => {
    if (previewId !== null && (previewClip === null || route !== "history")) {
      setPreviewId(null);
    }
  }, [previewId, previewClip, route]);

  // Reading the scheme is kept here rather than in the pane, so the preview's
  // colour row never shows stale swatches after the eyedropper adds one. The
  // backend caches the extracted list, so switching back to a clip is free.
  const schemeClipId = selectedClip?.kind === "image" ? selectedClip.id : null;
  useEffect(() => {
    setScheme(null);
    setSchemeError(null);
    if (schemeClipId === null) {
      setSchemeBusy(false);
      return;
    }
    let cancelled = false;
    setSchemeBusy(true);
    ensureColorScheme(schemeClipId)
      .then((colors) => {
        if (!cancelled) setScheme({ clipId: schemeClipId, colors });
      })
      .catch((e) => {
        if (!cancelled) setSchemeError(errorMessage(e));
      })
      .finally(() => {
        if (!cancelled) setSchemeBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [schemeClipId]);

  /** The loaded scheme, but only while it belongs to the clip on screen. */
  const schemeColors = scheme && scheme.clipId === selectedClip?.id ? scheme.colors : null;

  // ---- window behaviour ---------------------------------------------------

  // A screen sample finishes after the window has been restored, so route the
  // result into the Colors view and keep the sampled clip selected.
  useEffect(() => {
    // A pick that finished belongs to the history window; the settings window
    // showing the Colors view instead of its own page is the same bug as below.
    if (windowLabel !== "main") return;
    const pending = listen<ColorPickPayload>(COLOR_PICKED, (event) => {
      const { hex, id, error } = event.payload;
      // Cancelling a pick is a choice, not a failure: stay quiet.
      if (!hex) {
        if (error) setError(error);
        return;
      }
      setRoute("history");
      setView({ kind: "colors" });
      setQuery("");
      if (typeof id === "number") setSelectedId(id);
      setNotice(`Copied ${displayHex(hex)} and added it to Colors.`);
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  // A screenshot also finishes after the window is restored. Unlike a colour it
  // is an ordinary image clip, so show it in the full history and select it.
  useEffect(() => {
    // Screenshot and colour events are broadcast to every webview, so the guard is
    // what keeps a capture from repainting the settings window as the app.
    if (windowLabel !== "main") return;
    const pending = listen<ScreenshotPayload>(SCREENSHOT_CAPTURED, (event) => {
      const { id, error } = event.payload;
      if (error) {
        setError(error);
        return;
      }
      // No id means the user cancelled.
      if (typeof id !== "number") return;
      setRoute("history");
      setView({ kind: "all" });
      setQuery("");
      setSelectedId(id);
      setNotice("Captured a screenshot and added it to history.");
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    // Only the main window follows the backend's route: the settings and add-clip
    // windows are their own page, and a broadcast would repaint them as the app.
    if (windowLabel !== "main") return;
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
        // The settings window may have changed the palette or layout while this
        // window was in the background, so re-read them on the way back.
        refreshSettings();
        if (route === "history") searchRef.current?.focus();
      } else if (settings?.hideOnBlur && route !== "add-clip") {
        // The add-clip window holds a draft that has not been saved, so losing
        // focus must not take it off the screen.
        void appWindow.hide();
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [route, settings?.hideOnBlur, refreshSettings]);

  useEffect(() => {
    if (selectedId !== null) scrollRowIntoView(selectedId);
  }, [selectedId]);

  // ---- actions ------------------------------------------------------------

  const activate = useCallback(async (clip: ClipSummary) => {
    try {
      const close = settings?.closeOnDoubleClick ?? true;
      // The backend decides whether to also paste, and hides the window unless the
      // user asked to keep it open after activating a clip.
      await copyClip(clip.id, undefined, close);
      if (!close) setNotice(`Copied “${clip.preview.slice(0, 40) || "clip"}”.`);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [settings?.closeOnDoubleClick]);

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

  /** Marks a text clip as a secret, or clears the mark. */
  /**
   * Whether sealed clips can be read right now. Asked once per load, because a
   * restart always comes back locked and the interface has to look different.
   */
  const [vaultStatus, setVaultStatus] = useState<VaultStatus>("none");
  const [vaultPrompt, setVaultPrompt] = useState<
    { kind: "create" | "unlock" | "change"; reason?: string; then?: () => void } | null
  >(null);

  useEffect(() => {
    vaultStatusCall().then(setVaultStatus).catch(() => setVaultStatus("none"));
  }, []);

  /**
   * Runs `action` if sealed clips can be read, and otherwise asks for the
   * passphrase first and then runs it.
   */
  const withVault = useCallback(
    (action: () => void, reason: string) => {
      if (vaultStatus === "unlocked") {
        action();
        return;
      }
      setVaultPrompt({
        kind: vaultStatus === "none" ? "create" : "unlock",
        reason,
        then: action,
      });
    },
    [vaultStatus],
  );

  const applySecret = useCallback(
    async (clip: ClipSummary, secret: boolean) => {
      try {
        await setClipSecret(clip.id, secret);
        bump();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

  const toggleSecret = useCallback(
    (clip: ClipSummary) => {
      // Unmarking only needs the key that is already in the database, so it can
      // happen while locked; marking has to encrypt, which needs the vault open.
      if (clip.secretLevel || vaultStatus === "unlocked") {
        void applySecret(clip, !clip.secretLevel);
        return;
      }
      withVault(
        () => void applySecret(clip, true),
        "Marking a clip as secret encrypts it, so the vault has to be open first.",
      );
    },
    [applySecret, vaultStatus, withVault],
  );

  /** The four things the passphrase window can be asked to do. */
  const runVaultPrompt = useCallback(
    async ({ passphrase, next }: { passphrase: string; next?: string }) => {
      const kind = vaultPrompt?.kind ?? "unlock";
      try {
        if (kind === "create") {
          await createVault(passphrase);
        } else if (kind === "change") {
          await changeVaultPassphrase(passphrase, next ?? "");
        } else {
          await unlockVault(passphrase);
        }
      } catch (e) {
        // The window shows this itself; closing it here would lose the message.
        throw e;
      }
      setVaultStatus("unlocked");
      const follow = vaultPrompt?.then;
      setVaultPrompt(null);
      bump();
      follow?.();
    },
    [bump, vaultPrompt],
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

  /** Files clips dropped from the history onto a collection or archive. */
  const dropClipsOnCollection = useCallback(
    async (ids: number[], collection: Collection) => {
      try {
        const changed = await addToCollection(ids, collection.id);
        bump();
        setNotice(
          changed === 1
            ? `Filed 1 clip in “${collection.name}”.`
            : `Filed ${changed} clips in “${collection.name}”.`,
        );
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

  /** Creates a collection and immediately moves it into the archives. */
  const addArchive = useCallback(
    async (name: string) => {
      try {
        const created = await createCollection(name);
        await setCollectionArchived(created.id, true);
        bump();
        setView({ kind: "collection", id: created.id, name: created.name });
        setNotice(`Created the “${created.name}” archive.`);
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

  /** Opens an email clip in the user's default mail client. */
  const openEmailClient = useCallback((email: string) => {
    openEmail(email).catch((e) => setError(errorMessage(e)));
  }, []);

  /** Copies a generated scheme colour and files it in history. */
  const copySchemeColor = useCallback(
    (hex: string) => {
      copyColor(hex)
        .then(() => {
          bump();
          setNotice(`Copied ${displayHex(hex)} and added it to Colors.`);
        })
        .catch((e) => setError(errorMessage(e)));
    },
    [bump],
  );

  /**
   * Adds the spot the preview's eyedropper picked to the clip's scheme, and keeps
   * the shared list in step so the properties pane shows it too.
   */
  const addPickedColor = useCallback(async (clipId: number, x: number, y: number) => {
    try {
      const picked = await addSchemeColor(clipId, x, y);
      setScheme({ clipId, colors: picked.colors });
      setNotice(
        picked.added
          ? `Added ${displayHex(picked.hex)} to the color scheme.`
          : `${displayHex(picked.hex)} is already in the color scheme.`,
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  /** Saves every colour in a generated scheme into a new palette archive. */
  const savePalette = useCallback(
    async (colors: string[]) => {
      try {
        const archive = await saveColorPalette(colors);
        bump();
        setNotice(
          `Saved ${colors.length} color${colors.length === 1 ? "" : "s"} to “${archive.name}”.`,
        );
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
  );

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
      else if (action.kind === "open-email") openEmailClient(target);
      else showPath(target);
    },
    [openLink, openEmailClient, showPath],
  );

  // ---- utilities ----------------------------------------------------------

  /** Picks the WAV played for a copy outcome; cancelling keeps the current file. */
  const chooseSound = useCallback(
    async (kind: "copy" | "fail") => {
      try {
        const chosen = await openDialog({
          multiple: false,
          title: kind === "copy" ? "Choose a copy sound" : "Choose a failure sound",
          filters: [{ name: "WAV audio", extensions: ["wav"] }],
        });
        if (typeof chosen !== "string") return;
        await applyPatch(
          kind === "copy" ? { copySoundFile: chosen } : { copyFailSoundFile: chosen },
        );
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [applyPatch],
  );

  const showError = useCallback((text: string) => setError(text), []);  const showNotice = useCallback((text: string) => setNotice(text), []);

  /** Hides the window and samples the next clicked pixel. */
  const pickColor = useCallback(() => {
    pickScreenColor().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Hides the window and captures the rectangle the user drags. */
  const captureRegion = useCallback(() => {
    captureScreenRegion().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Hides the window and captures the window the user clicks. */
  const captureWindow = useCallback(() => {
    captureScreenWindow().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Hides the window and captures every monitor at once. */
  const captureScreen = useCallback(() => {
    captureWholeScreen().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Picks a window, scrolls it to the end and captures the whole length. */
  const captureScrolling = useCallback(() => {
    captureScrollingWindow().catch((e) => setError(errorMessage(e)));
  }, []);

  /** Remembers which capture type the header button should offer next time. */
  const rememberCaptureMode = useCallback(
    (captureMode: CaptureMode) => {
      void applyPatch({ captureMode });
    },
    [applyPatch],
  );

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

  const commitEmojiScale = useCallback(
    (next: number) => {
      void applyPatch({ emojiScale: next });
    },
    [applyPatch],
  );

  const commitGifLoop = useCallback(
    (next: boolean) => {
      void applyPatch({ gifLoop: next });
    },
    [applyPatch],
  );

  /**
   * Adds or removes an image clip on the GIF shelf.
   *
   * Saving also files the clip into the hidden GIF archive, so its media stays
   * readable after it leaves the history; taking it off the shelf releases it.
   */
  const toggleClipGif = useCallback(
    async (clip: ClipSummary) => {
      const current = settings?.gifFavorites ?? [];
      const id = `clip:${clip.id}`;
      const saved = current.some((favorite) => favorite.id === id);
      try {
        await setClipSavedGif(clip.id, !saved);
      } catch (e) {
        setError(errorMessage(e));
        return;
      }
      const next = saved
        ? current.filter((favorite) => favorite.id !== id)
        : [
            {
              id,
              title: (clip.preview || "Image clip").slice(0, 120),
              url: "",
              previewUrl: "",
              stillUrl: "",
              clipId: clip.id,
            },
            ...current,
          ];
      await applyPatch({ gifFavorites: next.slice(0, 100) });
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
      // A click moves the keyboard into the list, so Space, the arrow keys and
      // Delete act on the selection instead of typing into the search field.
      if (document.activeElement === searchRef.current) searchRef.current?.blur();
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
        // The first Ctrl+click has to carry the clip that is already highlighted
        // into the group, otherwise it silently drops out of the selection.
        setSelectedIds((current) => {
          const anchor =
            current.length === 0 &&
            selectedId !== null &&
            selectedId !== id &&
            items.some((item) => item.id === selectedId)
              ? selectedId
              : null;
          const base = anchor === null ? current : [anchor];
          return base.includes(id)
            ? base.filter((existing) => existing !== id)
            : [...base, id];
        });
        setSelectedId(id);
        return;
      }
      setSelectedIds([]);
      setSelectedId(id);
    },
    [items, selectedId],
  );

  const clearSelection = useCallback(() => setSelectedIds([]), []);

  /** Resolves the Markdown vault, asking for a folder while the setting is empty. */
  const markdownVault = useCallback(async () => {
    const stored = settings?.markdownExportDir.trim();
    if (stored) return stored;
    const chosen = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose a folder for Markdown notes",
    });
    return typeof chosen === "string" ? chosen : null;
  }, [settings?.markdownExportDir]);

  /** Writes the notes and reports the folder; a failure keeps the overlay open. */
  const writeMarkdown = useCallback(
    async (ids: number[], dir: string, hide: boolean) => {
      try {
        const exported = await exportMarkdown(ids, dir);
        if (ids.length > 1) clearSelection();
        setNotice(markdownNotice(exported, dir));
        if (hide) await hideWindow();
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [clearSelection],
  );

  /**
   * Exports text clips as Markdown notes. A likely secret ends up as plain text
   * on disk, so those wait for an explicit answer before anything is written.
   */
  const exportNotes = useCallback(
    async (ids: number[], hide = false) => {
      if (ids.length === 0) return;
      let dir: string;
      try {
        const resolved = await markdownVault();
        if (resolved === null) return;
        dir = resolved;
      } catch (e) {
        setError(errorMessage(e));
        return;
      }
      const secrets = ids.filter(
        (id) => items.find((item) => item.id === id)?.secretLevel === "high",
      ).length;
      if (secrets > 0) {
        setPendingMarkdown({ ids, dir, hide, secrets });
        return;
      }
      await writeMarkdown(ids, dir, hide);
    },
    [items, markdownVault, writeMarkdown],
  );

  // The confirmation owns Escape while it is open, so the app underneath does
  // not hide as well.
  useEffect(() => {
    if (!pendingMarkdown) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setPendingMarkdown(null);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [pendingMarkdown]);

  const picked = useMemo(
    () => (selectedIds.length > 1 ? selectedIds : []),
    [selectedIds],
  );

  // ---- color scheme -------------------------------------------------------

  /** The colour clips in the list on screen, in the order it shows them. */
  const colorClips = useMemo(
    () => items.filter((item) => item.color !== null),
    [items],
  );

  /**
   * Whether the Copy Color Scheme bar belongs in this view.
   *
   * Only inside an archive holding more than one colour clip: that is the case the
   * bar was asked for — a palette spread over several clips. Everywhere else (all
   * clips, text, the Colors section, a plain collection) it would be a button that
   * means something different from what the list is for.
   */
  const showColorSchemeBar = useMemo(() => {
    if (view.kind !== "collection" || colorClips.length < 2) return false;
    const collection = collections.find((entry) => entry.id === view.id);
    return collection?.archived === true;
  }, [collections, colorClips.length, view]);

  /**
   * Whether the selection can be written as Markdown notes.
   *
   * Only text clips have anything to write, so offering the action for a pile of
   * screenshots is a button that quietly does nothing.
   */
  const canExportMarkdown = useMemo(
    () => picked.some((id) => items.find((item) => item.id === id)?.kind === "text"),
    [items, picked],
  );

  /** Copies every colour in the list as a scheme: one uppercase hex per line. */
  const copyColorScheme = useCallback(() => {
    const text = colorSchemeText(colorClips);
    if (!text) return;
    copyText(text)
      .then(() => {
        const count = text.split("\n").length;
        setNotice(`Copied ${count} color${count === 1 ? "" : "s"} as a scheme.`);
      })
      .catch((e) => setError(errorMessage(e)));
  }, [colorClips]);

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
   * Creates a collection and files `ids` into it in one step.
   *
   * The right-click menu offers this so a new collection can be made for the clip
   * under the cursor without leaving the list.
   */
  const collectIntoNew = useCallback(
    async (ids: number[], name: string) => {
      const typed = name.trim();
      if (!typed) return;
      try {
        const created = await createCollection(typed);
        await addToCollection(ids, created.id);
        bump();
        setNotice(
          ids.length === 1
            ? `Created “${created.name}” and filed 1 clip in it.`
            : `Created “${created.name}” and filed ${ids.length} clips in it.`,
        );
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [bump],
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

  /**
   * Selects a clip and makes sure its properties pane is showing.
   *
   * Reads the stored setting rather than the derived flag, because the menu is
   * built further up the component than that flag exists. The pane can also be
   * collapsed, and expanding it is the most the list can do about that; the user
   * ends up in the one place the command promises.
   */
  const showProperties = useCallback(
    (id: number) => {
      setSelectedId(id);
      setSelectedIds([id]);
      if (settings?.detailPaneOpen === false) {
        void applyPatch({ detailPaneOpen: true });
      }
    },
    [applyPatch, settings?.detailPaneOpen],
  );

  const openClipMenu = useCallback(
    (clip: ClipSummary, event: ReactMouseEvent) => {
      event.preventDefault();
      const items: ContextMenuItem[] = [];
      // Right-clicking inside the selection acts on all of it; right-clicking
      // elsewhere falls back to the single clip under the cursor.
      const selection = picked.includes(clip.id) ? picked : [];
      const targets = selection.length > 1 ? selection : [clip.id];
      const single = selection.length > 1;

      if (single) {
        const count = selection.length;
        items.push({
          label: `Pin ${count} clips`,
          icon: <IconPin className="size-3" />,
          onSelect: () => void bulkPin(true),
        });
        items.push({
          label: `Unpin ${count} clips`,
          icon: <IconPin className="size-3" />,
          onSelect: () => void bulkPin(false),
        });
        items.push({
          label: `Tag ${count} clips…`,
          icon: <IconTag className="size-3" />,
          onSelect: () => setPrompt({ kind: "bulk-tag" }),
        });
        items.push({
          label: `Collect ${count} clips…`,
          icon: <IconFolder className="size-3" />,
          onSelect: () => setPrompt({ kind: "bulk-collect" }),
        });
        items.push({
          label: `Save ${count} clips to…`,
          icon: <IconSave className="size-3" />,
          onSelect: () => void bulkExport(),
        });
        if (canExportMarkdown) {
          items.push({
            label: `Export ${count} as Markdown`,
            icon: <IconSave className="size-3" />,
            onSelect: () => void exportNotes(selection),
          });
        }
        items.push({
          label: `Delete ${count} clips`,
          icon: <IconTrash className="size-3" />,
          onSelect: () => void bulkDelete(),
        });
      } else {
        // Copy is the most common action, so it leads the single-clip menu.
        items.push({
          label: "Copy",
          icon: <IconCopy className="size-3" />,
          onSelect: () => void copySilently(clip),
        });
      }

      const group = (item: ContextMenuItem): ContextMenuItem =>
        single ? { ...item, separatorBefore: true } : item;

      const collectionItems: ContextMenuItem[] = collections
        .filter((collection) => !collection.archived)
        .map((collection) => ({
          label: collection.name,
          icon: <IconFolder className="size-3" />,
          onSelect: () => void dropClipsOnCollection(targets, collection),
        }));
      const archiveItems: ContextMenuItem[] = collections
        .filter((collection) => collection.archived)
        .map((collection) => ({
          label: collection.name,
          icon: <IconArchive className="size-3" />,
          onSelect: () => void dropClipsOnCollection(targets, collection),
        }));

      items.push(
        group({
          label: "Add to Collection",
          icon: <IconFolder className="size-3" />,
          onSelect: () => undefined,
          // First, because making a collection for the clip you just right-clicked
          // is the common case when nothing suitable exists yet.
          submenu: [
            {
              label: "New collection…",
              icon: <IconPlus className="size-3" />,
              onSelect: () => {
                setNewCollectionTargets(targets);
                setPrompt({ kind: "new-collection" });
              },
            },
            ...collectionItems,
          ],
        }),
        {
          label: "Add to Archive",
          icon: <IconArchive className="size-3" />,
          onSelect: () => undefined,
          submenu: archiveItems,
        },
      );

      if (clip.contentKind === "url") {
        const url = clip.preview.trim();
        items.push(
          group({
            label: "Open link",
            icon: <IconExternal className="size-3" />,
            onSelect: () => openLink(url),
          }),
        );
      }
      if (clip.contentKind === "email") {
        const email = clip.preview.trim();
        items.push(
          group({
            label: "Send email",
            icon: <IconMail className="size-3" />,
            onSelect: () => openEmailClient(email),
          }),
        );
      }
      if (clip.contentKind === "path") {
        const path = clip.preview.trim();
        items.push(
          group({
            label: "Open",
            icon: <IconExternal className="size-3" />,
            onSelect: () => openFile(path),
          }),
        );
        items.push({
          label: "Show in Explorer",
          icon: <IconFolder className="size-3" />,
          onSelect: () => showPath(path),
        });
      }
      if (clip.kind !== "text") {
        items.push(
          group({
            label: clip.kind === "image" ? "Show image in Explorer" : "Show in Explorer",
            icon: clip.kind === "image" ? <IconImage className="size-3" /> : <IconFolder className="size-3" />,
            onSelect: () => {
              revealClip(clip.id).catch((e) => setError(errorMessage(e)));
            },
          }),
        );
      }
      if (clip.kind === "image" && clip.hasOcr) {
        items.push({
          label: "Copy text from image",
          icon: <IconText className="size-3" />,
          onSelect: () => void copyTextFromImage(clip),
        });
      }
      if (clip.kind === "text") {
        items.push(
          group({
            label: "Export as Markdown note…",
            icon: <IconSave className="size-3" />,
            onSelect: () => void exportNotes([clip.id]),
          }),
        );
      }
      // The two things the preview and the properties pane are for are worth
      // reaching without leaving the list, so they lead the per-clip group.
      items.push(
        group({
          label: "Quick preview",
          icon: <IconImage className="size-3" />,
          onSelect: () => setPreviewId(clip.id),
          // A file list has no single thing to show, which is exactly why Space
          // refuses it too.
          disabled: clip.kind === "files",
        }),
        {
          label: "Show properties",
          icon: <IconList className="size-3" />,
          onSelect: () => showProperties(clip.id),
        },
      );

      if (!single) {
        items.push(
          group({
            label: "Save to folder…",
            icon: <IconSave className="size-3" />,
            onSelect: () => void saveAs(clip),
          }),
          {
            label: clip.pinned ? "Unpin" : "Pin",
            icon: <IconPin className="size-3" filled={clip.pinned} />,
            onSelect: () => void togglePin(clip),
          },
        );
      }

      // Only text can be a secret, and only a text clip is worth marking.
      if (clip.kind === "text") {
        items.push(
          group({
            label: clip.secretLevel ? "Remove secret mark" : "Make secret",
            icon: <IconShield className="size-3" />,
            onSelect: () => toggleSecret(clip),
          }),
        );
      }

      items.push({
        label: single ? `Delete ${selection.length} clips` : "Delete",
        icon: <IconTrash className="size-3" />,
        separatorBefore: true,
        onSelect: () => {
          if (single) void bulkDelete();
          else void removeClip(clip);
        },
      });
      setMenu({ x: event.clientX, y: event.clientY, items });
    },
    [
      bulkDelete,
      bulkExport,
      bulkPin,
      collections,
      copySilently,
      copyTextFromImage,
      dropClipsOnCollection,
      exportNotes,
      canExportMarkdown,
      openEmailClient,
      openFile,
      openLink,
      picked,
      saveAs,
      showPath,
      showProperties,
      togglePin,
    ],
  );


  // ---- dragging clips out -------------------------------------------------

  // The payload of a clip lives in the local store, and a drag has to hand its
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
    // The sidebar reads this payload to file the drag onto a collection; the
    // standard flavours below keep a drop on another app working as before.
    const ids = picked.includes(clip.id) ? picked : [clip.id];
    event.dataTransfer.setData("application/x-copypaste-clips", JSON.stringify(ids));
    if (clip.kind === "text") {
      event.dataTransfer.setData("text/plain", loaded?.text ?? clip.preview);
    } else if (clip.kind === "image" && clip.imagePath) {
      event.dataTransfer.setData("text/uri-list", fileUri(clip.imagePath));
    } else if (clip.kind === "files" && loaded) {
      event.dataTransfer.setData("text/uri-list", loaded.filePaths.map(fileUri).join("\r\n"));
    }
  }, [picked]);


  // ---- appearance ---------------------------------------------------------

  const viewModes = settings?.viewModes ?? DEFAULT_VIEW_MODES;
  // Sidebar kinds map onto the stored slots; collections, tags and saved searches
  // share one.
  const viewKey: keyof ViewModes =
    view.kind === "collection" || view.kind === "tag" || view.kind === "search"
      ? "other"
      : view.kind === "content" || view.kind === "secrets"
        ? "text"
        : view.kind === "image" ||
            view.kind === "screenshots" ||
            view.kind === "people" ||
            view.kind === "animated"
        ? "images"
        : view.kind === "emoji"
          ? "all"
          : view.kind === "gifs"
            ? "other"
            : view.kind;
  const viewMode: ViewMode = viewModes[viewKey] === "grid" ? "grid" : "list";
  // Card size is remembered per sidebar view, so a collection or saved search can
  // be zoomed without touching the categories around it. The older `gridScale`
  // remains the fallback for views that have never been resized.
  const zoomKey =
    view.kind === "collection"
      ? `collection:${view.id}`
      : view.kind === "tag"
        ? `tag:${view.name}`
        : view.kind === "search"
          ? `search:${view.name}`
          : view.kind === "content"
            ? `content:${view.contentKind}`
            : view.kind;
  const gridScale = settings?.gridScales?.[zoomKey] ?? settings?.gridScale ?? 1;
  const textScale = settings?.textScale ?? 100;
  const detailOpen = settings?.detailPaneOpen ?? true;
  const sortOrder = settings?.sortOrder ?? "updated-desc";

  // While the slider is moving the grid follows a local value, so resizing is
  // instant; the setting catches up once the gesture pauses. The key is captured
  // with the value so a pending write cannot land on a view the user switched to.
  const [liveGridScale, setLiveGridScale] = useState<{ key: string; percent: number } | null>(
    null,
  );
  const gridPercent =
    liveGridScale?.key === zoomKey ? liveGridScale.percent : Math.round(gridScale * 100);

  const changeGridScale = useCallback(
    (percent: number) => setLiveGridScale({ key: zoomKey, percent }),
    [zoomKey],
  );

  useEffect(() => {
    if (liveGridScale === null) return;
    const timer = window.setTimeout(() => {
      void applyPatch({
        gridScales: {
          ...(settings?.gridScales ?? {}),
          [liveGridScale.key]: liveGridScale.percent / 100,
        },
      });
    }, 300);
    return () => window.clearTimeout(timer);
  }, [liveGridScale, applyPatch, settings?.gridScales]);

  // Once the stored value matches the drag, hand control back to the settings.
  useEffect(() => {
    if (liveGridScale === null) return;
    const stored =
      settings?.gridScales?.[liveGridScale.key] ?? settings?.gridScale ?? 1;
    if (Math.round(stored * 100) === liveGridScale.percent) setLiveGridScale(null);
  }, [settings?.gridScales, settings?.gridScale, liveGridScale]);

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
      const previewOpen = previewClip !== null;
      // Space is a character everywhere text is typed, including the search
      // field, which otherwise doubles as list navigation.
      const typing =
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        target?.isContentEditable === true;

      if (event.key === "Escape") {
        event.preventDefault();
        if (previewOpen) {
          setPreviewId(null);
          return;
        }
        if (selectedIds.length > 0) {
          setSelectedIds([]);
          return;
        }
        if (inSearch && query) {
          setQuery("");
          return;
        }
        if (route === "settings") {
          if (standaloneSettings) void getCurrentWindow().close();
          else setRoute("history");
          return;
        }
        if (route === "add-clip") {
          void getCurrentWindow().close();
          return;
        }
        void hideWindow();
        return;
      }

      // Space opens the preview for a single clip and closes it again. It never
      // reaches this branch while a field has focus, so spaces still type.
      if ((event.key === " " || event.code === "Space") && !event.repeat) {
        if (typing || inSearch || route !== "history") return;
        if (previewOpen) {
          event.preventDefault();
          setPreviewId(null);
          return;
        }
        // A group selection has no single clip to preview.
        if (selectedClip && selectedIds.length < 2 && selectedClip.kind !== "files") {
          event.preventDefault();
          setPreviewId(selectedClip.id);
        }
        return;
      }
      // While the preview covers the list, no other shortcut reaches it. Escape
      // and Space were handled above, and the field guard below lets text areas
      // inside the preview keep their own keys.
      if (previewOpen) return;

      // Keystrokes in a real form control stay there; only the search field doubles
      // as list navigation.
      if ((tag === "INPUT" || tag === "TEXTAREA") && !inSearch) return;
      // The settings window has no list of its own to act on.
      if (route !== "history") return;

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

      // Ctrl+Shift+M exports the selection as Markdown notes: it writes, names
      // the folder in the notice, then hides the overlay.
      if (
        (event.ctrlKey || event.metaKey) &&
        event.shiftKey &&
        event.key.toLowerCase() === "m"
      ) {
        const ids =
          picked.length > 1
            ? picked
            : selectedClip?.kind === "text"
              ? [selectedClip.id]
              : [];
        if (ids.length === 0) return;
        event.preventDefault();
        void exportNotes(ids, true);
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
    previewClip,
    query,
    route,
    activate,
    bulkDelete,
    exportNotes,
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

  // The add-clip window is a single small form: none of the history chrome, list
  // shortcuts or sidebar belong in it.
  if (route === "add-clip") return <AddClipForm />;

  return (
    <VaultGateProvider
      value={{
        locked: vaultStatus !== "unlocked",
        requestUnlock: () =>
          setVaultPrompt({ kind: vaultStatus === "none" ? "create" : "unlock" }),
      }}
    >
    <div className="relative flex h-full flex-col bg-canvas">
      {!standaloneSettings && (
        <TitleBar
          subtitle={
            route === "settings" ? "Settings" : `${activeFilter}${query ? ` · “${query}”` : ""}`
          }
          settingsOpen={route === "settings"}
          onToggleSettings={() =>
            openSettingsWindow().catch((e) => setError(errorMessage(e)))
          }
          showCaptureIcons={settings?.showTitlebarCaptureIcons ?? true}
          onPickColor={pickColor}
          captureMode={settings?.captureMode ?? "region"}
          onRememberCaptureMode={rememberCaptureMode}
          onCaptureRegion={captureRegion}
          onCaptureWindow={captureWindow}
          onCaptureScreen={captureScreen}
          onCaptureScrolling={captureScrolling}
          onHide={() => void hideWindow()}
        />
      )}

      {(banner || info) && (
        <div className="pointer-events-none absolute inset-x-0 top-11 z-40 flex justify-center px-3">
          <div
            role={banner ? "alert" : "status"}
            className={`pointer-events-auto flex w-full max-w-[680px] items-start gap-2 rounded-panel border px-3 py-1.5 text-[11px] shadow-lg animate-rise ${
              banner
                ? "border-danger/40 bg-[color-mix(in_srgb,var(--color-panel)_92%,var(--color-danger))] text-danger"
                : "border-ok/40 bg-[color-mix(in_srgb,var(--color-panel)_92%,var(--color-ok))] text-ok"
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
          onRunSetup={() => setWizardOpen(true)}
          onOpenChangelog={(version) => openLink(releaseUrl(version))}
          onOpenProject={() => openLink(projectUrl())}
          onChooseSound={(kind) => void chooseSound(kind)}
          onPatch={applyPatch}
          onDismissError={() => setError(null)}
          vaultStatus={vaultStatus}
          onVaultPrompt={(kind) => setVaultPrompt({ kind })}
          onLockVault={() => {
            void lockVault()
              .then(() => {
                setVaultStatus("locked");
                bump();
              })
              .catch((e) => setError(errorMessage(e)));
          }}
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
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {view.kind !== "gifs" && view.kind !== "emoji" && (
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
                onAddClip={() =>
                  openAddClipWindow().catch((e) => setError(errorMessage(e)))
                }
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
                  {canExportMarkdown && (
                    <button
                      type="button"
                      onClick={() => void exportNotes(picked)}
                      title="Write the selected text clips as Markdown notes"
                      className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                    >
                      Markdown
                    </button>
                  )}
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

            </>
          )}

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
              onCreateArchive={(name) => void addArchive(name)}
              onDeleteCollection={(collection) => void removeCollection(collection)}
              onArchiveCollection={(collection, archived) =>
                void archiveCollection(collection, archived)
              }
              onContextMenuCollection={openArchiveMenu}
              onContextMenuTag={openTagMenu}
              onContextMenuSearch={openSearchMenu}
              listLimit={settings?.sidebarListLimit ?? 6}
              onDropClips={(ids, collection) => void dropClipsOnCollection(ids, collection)}
              textExpanded={settings?.textExpanded ?? false}
              onToggleText={() => void applyPatch({ textExpanded: !(settings?.textExpanded ?? false) })}
              imagesExpanded={settings?.imagesExpanded ?? false}
              onToggleImages={() =>
                void applyPatch({ imagesExpanded: !(settings?.imagesExpanded ?? false) })
              }
              collapsedSections={settings?.sidebarCollapsed ?? []}
              onToggleSection={toggleSidebarSection}
              open={settings?.sidebarOpen ?? true}
              onToggleOpen={() => void applyPatch({ sidebarOpen: !(settings?.sidebarOpen ?? true) })}
              onPickColor={pickColor}
              onCaptureRegion={captureRegion}
              onCaptureWindow={captureWindow}
              onCaptureScreen={captureScreen}
              onCaptureScrolling={captureScrolling}
              sectionOrder={settings?.sidebarSectionOrder ?? []}
              onSectionOrderChange={(order) => void applyPatch({ sidebarSectionOrder: order })}
            />

            <div className="flex min-w-0 flex-1 flex-col">
              {view.kind === "gifs" ? (
                <GifBrowser
                  favorites={settings?.gifFavorites ?? []}
                  hasApiKey={Boolean(settings?.giphyApiKey.trim())}
                  scale={settings?.gifScale ?? 1}
                  onScaleCommit={commitGifScale}
                  loop={settings?.gifLoop ?? true}
                  onLoopChange={commitGifLoop}
                  onFavoritesChange={changeGifFavorites}
                  onCopyClip={(id) => void copyClipById(id)}
                  onNotice={showNotice}
                  onError={showError}
                />
              ) : view.kind === "emoji" ? (
                <EmojiBrowser
                  favorites={settings?.emojiFavorites ?? []}
                  scale={settings?.emojiScale ?? 1}
                  onScaleCommit={commitEmojiScale}
                  onCopy={copyEmoji}
                  onContextMenu={openEmojiMenu}
                />
              ) : (
                <div className="flex min-h-0 flex-1">
                  <div className="flex min-h-0 min-w-0 flex-1 flex-col">
                    {/* Above the clips and inside their column, so this reads as part
                        of the list rather than as a banner over the whole window. */}
                    {showColorSchemeBar && (
                      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-edge-soft bg-panel-2 px-3 py-1.5 text-[10px] text-ink-faint">
                        <span className="shrink-0 whitespace-nowrap tabular-nums">
                          {colorClips.length} colors in this archive
                        </span>
                        <span className="flex-1" />
                        <button
                          type="button"
                          onClick={copyColorScheme}
                          title="Copy every color in this archive as hex, one per line"
                          className="shrink-0 whitespace-nowrap rounded border border-edge bg-panel px-1.5 py-0.5 text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                        >
                          Copy Color Scheme
                        </button>
                      </div>
                    )}

                    {viewMode === "grid" ? (
                      <ClipGrid
                        items={items}
                        selectedId={selectedId}
                        selectedIds={selectedIds}
                        loading={loading}
                        emptyMessage={emptyMessage}
                        now={now}
                        scale={gridPercent / 100}
                        tagColors={tagColors}
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
                        showContent={archiveDisplay.cardContent}
                        showNotes={archiveDisplay.cardNotes}
                      />
                    ) : (
                      <ClipList
                        items={items}
                        selectedId={selectedId}
                        selectedIds={selectedIds}
                        loading={loading}
                        emptyMessage={emptyMessage}
                        now={now}
                        tagColors={tagColors}
                        onSelect={selectClip}
                        onActivate={(clip) => void activate(clip)}
                        onCopy={(clip) => void copySilently(clip)}
                        onContextMenu={openClipMenu}
                        onTogglePin={(clip) => void togglePin(clip)}
                        onDelete={(clip) => void removeClip(clip)}
                        onRunAction={runClipAction}
                        onPrefetch={prefetch}
                        onDragStart={startDrag}
                        showContent={archiveDisplay.listContent}
                        showNotes={archiveDisplay.listNotes}
                      />
                    )}
                  </div>

                  {/* The pane is useful in both layouts: card view still benefits from
                      tags, collections and the full preview. */}
                  <ClipDetailPane
                    clip={selectedClip}
                    detail={detail}
                    loadingDetail={loadingDetail}
                    collections={collections}
                    listLimit={settings?.sidebarListLimit ?? 6}
                    now={now}
                    tagColors={tagColors}
                    open={detailOpen}
                    sectionOrder={settings?.detailSectionOrder ?? []}
                    onSectionOrderChange={(order) => void applyPatch({ detailSectionOrder: order })}
                    onToggle={toggleDetail}
                    onCopy={(clip) => void activate(clip)}
                    onCopyText={(clip) => void copyTextFromImage(clip)}
                    onCopyColor={copySchemeColor}
                    onSavePalette={(colors) => void savePalette(colors)}
                    schemeColors={schemeColors}
                    schemeBusy={schemeBusy}
                    schemeError={schemeError}
                    onTogglePin={(clip) => void togglePin(clip)}
                    onDelete={(clip) => void removeClip(clip)}
                    onTagsCommit={(clip, next) => void commitTags(clip, next)}
                    onToggleSecret={(clip) => void toggleSecret(clip)}
                    onCollectionsChange={(clip, ids) => void commitCollections(clip, ids)}
                    onSaveAs={(clip) => void saveAs(clip)}
                    onExportMarkdown={(clip) => void exportNotes([clip.id])}
                    onOpenUrl={openLink}
                    onOpenEmail={openEmailClient}
                    onOpenPath={openFile}
                    onRevealPath={showPath}
                    archives={collections.filter((collection) => collection.archived)}
                    width={settings?.detailPaneWidth ?? 286}
                    onWidthChange={() => undefined}
                    onWidthCommit={(next) => void applyPatch({ detailPaneWidth: Math.round(next) })}
                    onArchiveChange={(clip, archiveId) => void commitArchive(clip, archiveId)}
                    onToggleGif={(clip) => void toggleClipGif(clip)}
                    gifFavorite={(settings?.gifFavorites ?? []).some(
                      (favorite) => favorite.id === `clip:${selectedClip?.id}`,
                    )}
                    onNoteCommit={(clip, note) => void commitNote(clip, note)}
                    onTextCommit={(clip, text) => void commitText(clip, text)}
                  />
                </div>
              )}
            </div>
          </div>

          {vaultPrompt && (
        <VaultPrompt
          kind={vaultPrompt.kind}
          reason={vaultPrompt.reason}
          onSubmit={runVaultPrompt}
          onClose={() => setVaultPrompt(null)}
        />
      )}

      {previewClip && (
            <ClipPreviewOverlay
              key={previewClip.id}
              clip={previewClip}
              detail={detail?.id === previewClip.id ? detail : null}
              schemeColors={schemeColors}
              schemeBusy={schemeBusy}
              onCopyColor={copySchemeColor}
              onSavePalette={(colors) => void savePalette(colors)}
              onPickColor={(x, y) => addPickedColor(previewClip.id, x, y)}
              onClose={() => setPreviewId(null)}
              onTextCommit={(clip, text) => void commitText(clip, text)}
              textScale={settings?.previewTextScale ?? 100}
              onTextScale={(next) => void applyPatch({ previewTextScale: next })}
            />
          )}
        </main>
      )}

      <StatusBar
        stats={stats}
        settings={settings}
        version={appVersion}
        checkingForUpdates={updater.state.kind === "checking"}
        onCheckForUpdates={() => void updater.checkNow()}
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
                      : prompt.kind === "new-collection"
                        ? "New collection"
                        : prompt.kind === "rename-search"
                          ? `Rename “${prompt.search.name}”`
                          : "New collection"
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
                    : prompt.kind === "new-collection"
                      ? "The collection is created and these clips are filed in it."
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
          placeholder={prompt.kind === "new-collection" ? "Collection name" : undefined}
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
            else if (current.kind === "bulk-collect") void bulkCollectByName(value);
            else if (current.kind === "new-collection")
              void collectIntoNew(newCollectionTargets, value);
            else if (current.kind === "save-search") void saveSearch(value);
            else if (current.kind === "rename-search") void renameSearch(current.search, value);
          }}
        />
      )}

      {pendingMarkdown && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 p-6 animate-fade-in"
          onMouseDown={() => setPendingMarkdown(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="markdown-secret-title"
            onMouseDown={(event) => event.stopPropagation()}
            className="w-full max-w-[330px] rounded-panel border border-edge bg-panel p-4 shadow-xl animate-rise"
          >
            <h2 id="markdown-secret-title" className="text-[13px] font-semibold text-ink">
              {pendingMarkdown.secrets === 1
                ? "Write a likely secret to disk?"
                : `Write ${pendingMarkdown.secrets} likely secrets to disk?`}
            </h2>
            <p className="mt-1.5 text-[11px] leading-relaxed text-ink-dim">
              {pendingMarkdown.secrets === 1 ? "This clip is" : "These clips are"} flagged
              as a credential. A Markdown note is plain text, outside CopyPaste's
              history, in {pendingMarkdown.dir}.
            </p>
            <div className="mt-3 flex justify-end gap-2">
              <button
                type="button"
                autoFocus
                onClick={() => setPendingMarkdown(null)}
                className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const job = pendingMarkdown;
                  setPendingMarkdown(null);
                  void writeMarkdown(job.ids, job.dir, job.hide);
                }}
                className="rounded border border-danger bg-danger px-2.5 py-1 text-[11px] text-white transition-colors hover:bg-danger/90"
              >
                Export anyway
              </button>
            </div>
          </div>
        </div>
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

      {settings &&
        (wizardOpen || (!standaloneSettings && !settings.setupComplete)) && (
          <FirstRunWizard
            settings={settings}
            firstRun={!settings.setupComplete}
            onApply={patch}
            onClose={() => setWizardOpen(false)}
            onFinish={() => {
              setWizardOpen(false);
              window.setTimeout(() => searchRef.current?.focus(), 0);
            }}
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

      {dragging && (
        <div className="pointer-events-none absolute inset-0 z-50 flex items-center justify-center bg-canvas/80">
          <div className="rounded-panel border border-accent bg-panel px-6 py-4 text-center shadow-xl">
            <p className="text-[13px] font-medium text-ink">Drop to add to history</p>
            <p className="mt-1 text-[11px] text-ink-faint">
              Images, text files and file lists become clips.
            </p>
          </div>
        </div>
      )}
    </div>
    </VaultGateProvider>
  );
}
