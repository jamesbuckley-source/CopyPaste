/**
 * TypeScript mirrors of the Rust DTOs.
 *
 * Field names match the `#[serde(rename_all = "camelCase")]` shape the backend
 * emits, so these interfaces can be trusted as-is at the IPC boundary.
 */

export type ClipKind = "text" | "image" | "files";

export interface ClipSummary {
  id: number;
  kind: ClipKind;
  preview: string;
  byteSize: number;
  sourceApp: string | null;
  pinned: boolean;
  /** Unix milliseconds. */
  createdAt: number;
  /** Unix milliseconds; advances whenever the clip is copied again. */
  updatedAt: number;
  copyCount: number;
  width: number | null;
  height: number | null;
  /** Media file names, turned into URLs by `mediaUrl`. */
  thumbFile: string | null;
  imageFile: string | null;
  fileCount: number;
  charCount: number;
  /** True when an image clip has recognised, non-empty text. */
  hasOcr: boolean;
  /** Normalised `#rrggbb` when the whole text payload is a colour code. */
  color: string | null;
  /** Derived shape of the payload: "color", "url", "email", "path" or "json". */
  contentKind: string | null;
  /** Absolute path of the stored image, for Explorer and drag targets. */
  imagePath: string | null;
  collectionIds: number[];
  tags: string[];
}

export interface ClipDetail extends ClipSummary {
  /** Present for text clips. */
  text: string | null;
  /** Present for file clips. */
  filePaths: string[];
  /** Text recognised in an image clip, when there is any. */
  ocrText: string | null;
  /** True when HTML/RTF were captured next to the plain text. */
  hasRich: boolean;
}

export interface RichText {
  html: string | null;
  rtf: string | null;
}

export interface ClipPage {
  items: ClipSummary[];
  total: number;
}

export interface ClipFilter {
  kind?: ClipKind | null;
  collectionId?: number | null;
  tag?: string | null;
  pinnedOnly?: boolean;
  colorsOnly?: boolean;
  /** Reach clips filed in archive collections, hidden by default. */
  includeArchived?: boolean;
  /** Substring match against the source application. */
  app?: string | null;
  /** Only clips captured before this Unix-millisecond timestamp. */
  before?: number | null;
  /** Exact `#rrggbb` match. */
  color?: string | null;
  /** Derived payload shape: "url", "email", "path" or "json". */
  contentKind?: string | null;
}

export interface Collection {
  id: number;
  name: string;
  color: string | null;
  /** Archive collections keep their clips out of history and bulk deletes. */
  archived: boolean;
  clipCount: number;
  createdAt: number;
}

export interface Tag {
  id: number;
  name: string;
  color: string | null;
  clipCount: number;
}

/** A named query kept in the settings and shown in the sidebar. */
export interface SavedSearch {
  name: string;
  query: string;
}

export interface Stats {
  total: number;
  pinned: number;
  images: number;
  files: number;
  /** Clips that only live in archive collections. */
  archived: number;
  /** Text clips whose payload is a single colour code. */
  colors: number;
  mediaBytes: number;
  databaseBytes: number;
  encryption: string;
}

/** Colour tokens mirrored from the backend theme settings. */
export interface ThemeColors {
  canvas: string;
  panel: string;
  panel2: string;
  panel3: string;
  edge: string;
  edgeSoft: string;
  ink: string;
  inkDim: string;
  inkFaint: string;
  accent: string;
  accentHi: string;
  danger: string;
}

export type ThemeMode = "dark" | "light" | "custom";

/** History ordering, mirrored by the backend's `sortOrder` setting. */
export type SortOrder = "updated-desc" | "updated-asc" | "created-desc" | "created-asc";

export type ViewMode = "list" | "grid";

/** Preferred layout per sidebar category. */
export interface ViewModes {
  all: ViewMode;
  text: ViewMode;
  images: ViewMode;
  files: ViewMode;
  colors: ViewMode;
  pinned: ViewMode;
  other: ViewMode;
}

export const DEFAULT_VIEW_MODES: ViewModes = {
  all: "list",
  text: "list",
  images: "grid",
  files: "list",
  colors: "grid",
  pinned: "list",
  other: "list",
};

export interface Theme {
  mode: ThemeMode;
  colors: ThemeColors;
}

export interface StorageInfo {
  dataDir: string;
  defaultDir: string;
  custom: boolean;
}

export interface Settings {
  hotkey: string;
  /** Shortcut that toggles the private capture pause. */
  pauseHotkey: string;
  /** Keep HTML/RTF alongside the plain body and paste it back. */
  pasteRichText: boolean;
  /** Empty the clipboard a while after a clip is reused. */
  autoClearClipboard: boolean;
  /** Seconds before the clipboard is emptied; 0 means as soon as the overlay hides. */
  autoClearDelaySecs: number;
  autoPaste: boolean;
  pasteDelayMs: number;
  launchAtLogin: boolean;
  startHidden: boolean;
  hideOnBlur: boolean;
  /** Keep the overlay above other applications. */
  alwaysOnTop: boolean;
  showInTaskbar: boolean;
  captureImages: boolean;
  captureFiles: boolean;
  preferImagesOverText: boolean;
  ignoreSensitive: boolean;
  ignoredApps: string[];
  maxItems: number;
  maxAgeDays: number;
  /** Sub-day retention; both age limits apply, 0 disables one. */
  maxAgeHours: number;
  maxTextBytes: number;
  maxImageBytes: number;
  pollIntervalMs: number;
  /** "list" or "grid", per history category. */
  viewModes: ViewModes;
  /** Card size multiplier for the grid view (0.6 – 2.0). */
  gridScale: number;
  /** Whether the right-hand preview pane is expanded. */
  detailPaneOpen: boolean;
  /** Interface scale in percent; 100 is the designed size (70 – 200). */
  textScale: number;
  sortOrder: SortOrder;
  theme: Theme;
  /** Check GitHub for a newer release; off disables the schedule entirely. */
  autoUpdate: boolean;
  /** Unix milliseconds of the last update check. */
  lastUpdateCheck: number;
  /** Saved queries shown as smart views in the sidebar. */
  savedSearches: SavedSearch[];
}

/** Sidebar selection, kept separate from the filter sent to the backend. */
export type SidebarView =
  | { kind: "all" }
  | { kind: "text" }
  | { kind: "image" }
  | { kind: "files" }
  | { kind: "colors" }
  | { kind: "pinned" }
  | { kind: "collection"; id: number; name: string }
  | { kind: "tag"; name: string }
  | { kind: "search"; name: string; query: string };

export function viewLabel(view: SidebarView): string {
  switch (view.kind) {
    case "all":
      return "All clips";
    case "text":
      return "Text";
    case "image":
      return "Images";
    case "files":
      return "Files";
    case "colors":
      return "Colours";
    case "pinned":
      return "Pinned";
    case "collection":
      return view.name;
    case "tag":
      return `#${view.name}`;
    case "search":
      return view.name;
  }
}

export function viewFilter(view: SidebarView): ClipFilter {
  switch (view.kind) {
    case "all":
      return {};
    case "text":
      return { kind: "text" };
    case "image":
      return { kind: "image" };
    case "files":
      return { kind: "files" };
    case "colors":
      return { colorsOnly: true };
    case "pinned":
      return { pinnedOnly: true };
    case "collection":
      return { collectionId: view.id };
    case "tag":
      return { tag: view.name };
    case "search":
      // A saved search is a query string, folded in by `parseSearch`.
      return {};
  }
}

/** The query a saved search should type into the search bar. */
export function viewQuery(view: SidebarView): string {
  return view.kind === "search" ? view.query : "";
}
