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
  /** True when the user attached a note to the clip. */
  hasNote: boolean;
  /** Normalised `#rrggbb` when the whole text payload is a color code. */
  color: string | null;
  /** Derived shape of the payload: "color", "url", "email", "path" or "json". */
  contentKind: string | null;
  /** Absolute path of the stored image, for Explorer and drag targets. */
  imagePath: string | null;
  /** True when the full animated GIF was recovered next to the still frame. */
  animated: boolean;
  collectionIds: number[];
  tags: string[];
  /** Confidence that the clip holds a credential: "low", "medium" or "high". */
  secretLevel: string | null;
  /** Faces the local detector found in an image clip; 0 otherwise. */
  faceCount: number;
}

export interface ClipDetail extends ClipSummary {
  /** Present for text clips. */
  text: string | null;
  /** Present for file clips. */
  filePaths: string[];
  /** Text recognised in an image clip, when there is any. */
  ocrText: string | null;
  /** A free-form note the user attached to the clip. */
  note: string | null;
  /** True when HTML/RTF were captured next to the plain text. */
  hasRich: boolean;
}

export interface RichText {
  html: string | null;
  rtf: string | null;
}

/** What a Markdown export wrote, and how many clips had no note body. */
export interface MarkdownExport {
  written: string[];
  skipped: number;
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
  /** Only clips flagged as credential-like. */
  secretsOnly?: boolean;
  /** Only image clips the detector found a face in. */
  facesOnly?: boolean;
  /** Only image clips backed by a recovered animation. */
  animatedOnly?: boolean;
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

/** One pinned entry in the GIF utility's top shelf. */
export interface GifFavorite {
  id: string;
  title: string;
  /** GIPHY animation address; empty for a clip saved from the history. */
  url: string;
  /** Smaller GIPHY animation used for the preview; empty for a clip. */
  previewUrl: string;
  /** Static frame used when the preview is not playing. */
  stillUrl: string;
  /** Set when the favourite is an image clip rather than a remote GIF. */
  clipId: number | null;
}

/** One GIPHY result, as the GIF browser needs it. */
export interface GifResult {
  id: string;
  title: string;
  /** Original animation, used when copying. */
  url: string;
  /** Smaller animation used for the grid. */
  previewUrl: string;
  /** Static frame shown while previews only animate on hover. */
  stillUrl: string;
  width: number;
  height: number;
}

export interface Stats {
  total: number;
  pinned: number;
  images: number;
  files: number;
  /** Clips that only live in archive collections. */
  archived: number;
  /** Text clips whose payload is a single color code. */
  colors: number;
  mediaBytes: number;
  databaseBytes: number;
}

/** Color tokens mirrored from the backend theme settings. */
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
  /** Look for faces in screenshots, so images can be filtered as People. */
  detectFaces: boolean;
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
  /** Card size overrides keyed by history subsection, so each keeps its zoom. */
  gridScales: Record<string, number>;
  /** Preview size multiplier for the emoji browser (0.6 – 2.0). */
  emojiScale: number;
  /** Close the overlay when a clip is activated with a double click. */
  closeOnDoubleClick: boolean;
  /** Whether the right-hand preview pane is expanded. */
  detailPaneOpen: boolean;
  /** Width of the preview pane in CSS pixels. */
  detailPaneWidth: number;
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
  /** Emoji pinned to the top of the emoji browser. */
  emojiFavorites: string[];
  /** Optional personal GIPHY key; the public test key is used when empty. */
  giphyApiKey: string;
  /** Play a short tick when a copy succeeds. */
  copySound: boolean;
  /** WAV for a successful copy; empty plays the bundled tick. */
  copySoundFile: string;
  /** Play a lower tone when a copy fails. */
  copyFailSound: boolean;
  /** WAV for a failed copy; empty plays the bundled failure sound. */
  copyFailSoundFile: string;
  /** Flag clips that look like they hold a credential. */
  flagSecrets: boolean;
  /** Also flag keyword-and-entropy matches. */
  secretHeuristics: boolean;
  /** Leave flagged secrets out of history entirely. */
  skipSecretCapture: boolean;
  /** GIFs pinned to the top of the GIF utility. */
  gifFavorites: GifFavorite[];
  /** GIF preview size multiplier (0.6 – 2.0). */
  gifScale: number;
  /** Keep GIF previews looping; off plays them only while hovered. */
  gifLoop: boolean;
  /** Folder one-click Markdown exports go to; empty asks each time. */
  markdownExportDir: string;
  /** Precede exported notes with YAML front matter. */
  markdownFrontMatter: boolean;
  /** Prefix note filenames with the capture date. */
  markdownDatePrefix: boolean;
  /** How many archives the sidebar lists before offering "Show all". */
  sidebarListLimit: number;
  /** Whether the left-hand filter sidebar is showing. */
  sidebarOpen: boolean;
  /** Whether the text payload sub-types are expanded in the sidebar. */
  textExpanded: boolean;
  /** Whether the image sub-types are expanded in the sidebar. */
  imagesExpanded: boolean;
  /** Sidebar sections the user collapsed; History is never collapsible. */
  sidebarCollapsed: string[];
  /** Sidebar section ids in the user's drag order; empty keeps the built-in one. */
  sidebarSectionOrder: string[];
  /** Preview-pane section ids in the user's drag order. */
  detailSectionOrder: string[];
  /** Per-archive content and note visibility. */
  archiveDisplay: ArchiveDisplay[];
  /** Set once the first-run wizard has been finished or skipped. */
  setupComplete: boolean;
}

/** Per-archive choices for what list rows and cards reveal. */
export interface ArchiveDisplay {
  archiveId: number;
  listContent: boolean;
  listNotes: boolean;
  cardContent: boolean;
  cardNotes: boolean;
}

export const DEFAULT_ARCHIVE_DISPLAY: Omit<ArchiveDisplay, "archiveId"> = {
  listContent: true,
  listNotes: true,
  cardContent: true,
  cardNotes: true,
};

/** One of the text payload shapes broken out under "Text". */
export type TextContentKind = "url" | "email" | "path" | "json" | "markdown";

export const TEXT_CONTENT_LABEL: Record<TextContentKind, string> = {
  url: "Link",
  email: "Email",
  path: "Path",
  json: "JSON",
  markdown: "Markdown",
};

/** Sidebar selection, kept separate from the filter sent to the backend. */
export type SidebarView =
  | { kind: "all" }
  | { kind: "text" }
  | { kind: "content"; contentKind: TextContentKind }
  | { kind: "secrets" }
  | { kind: "image" }
  | { kind: "screenshots" }
  | { kind: "people" }
  | { kind: "animated" }
  | { kind: "files" }
  | { kind: "colors" }
  | { kind: "pinned" }
  | { kind: "collection"; id: number; name: string }
  | { kind: "tag"; name: string }
  | { kind: "search"; name: string; query: string }
  | { kind: "emoji" }
  | { kind: "gifs" };

export function viewLabel(view: SidebarView): string {
  switch (view.kind) {
    case "all":
      return "All clips";
    case "text":
      return "Text";
    case "content":
      return TEXT_CONTENT_LABEL[view.contentKind];
    case "secrets":
      return "Secrets";
    case "image":
      return "Images";
    case "screenshots":
      return "Screenshots";
    case "people":
      return "People";
    case "animated":
      return "GIFs";
    case "files":
      return "Files";
    case "colors":
      return "Colors";
    case "pinned":
      return "Pinned";
    case "collection":
      return view.name;
    case "tag":
      return `#${view.name}`;
    case "search":
      return view.name;
    case "emoji":
      return "Emoji";
    case "gifs":
      return "My GIFs";
  }
}

export function viewFilter(view: SidebarView): ClipFilter {
  switch (view.kind) {
    case "all":
      return {};
    case "text":
      return { kind: "text" };
    case "content":
      return { kind: "text", contentKind: view.contentKind };
    case "secrets":
      return { secretsOnly: true };
    case "image":
      return { kind: "image" };
    case "screenshots":
      // Every capture this app takes is filed under the "Screenshot" source.
      return { kind: "image", app: "Screenshot" };
    case "people":
      return { kind: "image", facesOnly: true };
    case "animated":
      return { kind: "image", animatedOnly: true };
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
    case "emoji":
    case "gifs":
      return {};
  }
}

/** The query a saved search should type into the search bar. */
export function viewQuery(view: SidebarView): string {
  return view.kind === "search" ? view.query : "";
}
