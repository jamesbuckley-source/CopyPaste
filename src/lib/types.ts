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
  collectionIds: number[];
  tags: string[];
}

export interface ClipDetail extends ClipSummary {
  /** Present for text clips. */
  text: string | null;
  /** Present for file clips. */
  filePaths: string[];
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
}

export interface Collection {
  id: number;
  name: string;
  color: string | null;
  clipCount: number;
  createdAt: number;
}

export interface Tag {
  id: number;
  name: string;
  clipCount: number;
}

export interface Stats {
  total: number;
  pinned: number;
  images: number;
  files: number;
  mediaBytes: number;
  databaseBytes: number;
  encryption: string;
}

export interface Settings {
  hotkey: string;
  autoPaste: boolean;
  pasteDelayMs: number;
  launchAtLogin: boolean;
  startHidden: boolean;
  hideOnBlur: boolean;
  showInTaskbar: boolean;
  captureImages: boolean;
  captureFiles: boolean;
  preferImagesOverText: boolean;
  ignoreSensitive: boolean;
  ignoredApps: string[];
  maxItems: number;
  maxAgeDays: number;
  maxTextBytes: number;
  maxImageBytes: number;
  pollIntervalMs: number;
  /** "list" renders rows, "grid" renders cards. */
  viewMode: "list" | "grid";
  /** Card size multiplier for the grid view (0.6 – 2.0). */
  gridScale: number;
  /** Interface scale in percent; 100 is the designed size (70 – 200). */
  textScale: number;
}

/** Sidebar selection, kept separate from the filter sent to the backend. */
export type SidebarView =
  | { kind: "all" }
  | { kind: "text" }
  | { kind: "image" }
  | { kind: "files" }
  | { kind: "pinned" }
  | { kind: "collection"; id: number; name: string }
  | { kind: "tag"; name: string };

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
    case "pinned":
      return "Pinned";
    case "collection":
      return view.name;
    case "tag":
      return `#${view.name}`;
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
    case "pinned":
      return { pinnedOnly: true };
    case "collection":
      return { collectionId: view.id };
    case "tag":
      return { tag: view.name };
  }
}
