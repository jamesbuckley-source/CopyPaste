/**
 * Typed wrappers around the Tauri command surface.
 *
 * Everything the UI can do to the backend goes through here, so the rest of the
 * frontend never hand-builds an `invoke` payload. Argument names are camelCase
 * because Tauri maps snake_case Rust parameters onto camelCase keys on the wire.
 */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";

import type {
  ClipDetail,
  ClipFilter,
  ClipPage,
  Collection,
  GifFavorite,
  GifResult,
  MarkdownExport,
  RichText,
  Settings,
  Stats,
  StorageInfo,
  Tag,
} from "./types";

export type { GifResult };

/** Searches GIPHY; an empty query returns trending GIFs. */
export function searchGiphy(query: string, limit = 24): Promise<GifResult[]> {
  return invoke<GifResult[]>("search_giphy", { query, limit });
}

/** Downloads a GIPHY GIF and puts the file on the clipboard. */
export function copyGif(url: string, hideWindow = false): Promise<void> {
  return invoke("copy_gif", { url, hideWindow });
}

/** Copies a GIF file added to the utility earlier. `file` is its media name. */
export function copyCustomGif(file: string, hideWindow = true): Promise<void> {
  return invoke("copy_custom_gif", { file, hideWindow });
}

/** Copies a local GIF into the utility's media folder and returns a shelf entry. */
export function importCustomGif(path: string): Promise<GifFavorite> {
  return invoke<GifFavorite>("import_custom_gif", { path });
}

/** Marks or unmarks a clip as a secret from the properties pane. */
export function setClipSecret(id: number, secret: boolean): Promise<void> {
  return invoke("set_clip_secret", { id, secret });
}

/** Adds or clears the user's note on a clip. */
export function setClipNote(id: number, note: string | null): Promise<string | null> {
  return invoke<string | null>("set_clip_note", { id, note });
}

/** Replaces the body of a text clip after an inline edit. */
export function setClipText(id: number, text: string): Promise<void> {
  return invoke("set_clip_text", { id, text });
}

/** Minimises the window and waits for the next click to sample a screen colour. */
export function pickScreenColor(): Promise<void> {
  return invoke("pick_screen_color");
}

/** Hides the window and captures the rectangle the user drags on screen. */
export function captureScreenRegion(): Promise<void> {
  return invoke("capture_screen_region");
}

/** Hides the window and captures the window the user clicks. */
export function captureScreenWindow(): Promise<void> {
  return invoke("capture_screen_window");
}

/** Hides the window and captures every monitor at once. */
export function captureWholeScreen(): Promise<void> {
  return invoke("capture_whole_screen");
}

/** Picks a window, scrolls it to the end and captures the whole length. */
export function captureScrollingWindow(): Promise<void> {
  return invoke("capture_scrolling_window");
}

/** A shelf entry that points at a GIF rather than a stored clip. */
export function gifFavoriteFromResult(result: GifResult): GifFavorite {
  return {
    id: `giphy:${result.id}`,
    title: result.title,
    url: result.url,
    previewUrl: result.previewUrl,
    stillUrl: result.stillUrl,
    clipId: null,
  };
}

export interface ListArgs {
  filter?: ClipFilter;
  query?: string;
  limit?: number;
  offset?: number;
}

export function listClips({ filter, query, limit, offset }: ListArgs = {}): Promise<ClipPage> {
  return invoke<ClipPage>("list_clips", { filter, query, limit, offset });
}

export function getClip(id: number): Promise<ClipDetail> {
  return invoke<ClipDetail>("get_clip", { id });
}

export function getStats(): Promise<Stats> {
  return invoke<Stats>("get_stats");
}

/**
 * Puts a clip back on the clipboard. `autoPaste` overrides the stored setting for
 * this one action, and `hideWindow` lets a hover copy leave the overlay open.
 */
export function copyClip(id: number, autoPaste?: boolean, hideWindow = true): Promise<void> {
  return invoke("copy_clip", { id, autoPaste, hideWindow });
}

export function setPinned(id: number, pinned: boolean): Promise<void> {
  return invoke("set_pinned", { id, pinned });
}

/** Puts the text recognised in an image on the clipboard instead of the image. */
export function copyOcrText(id: number, hideWindow = true): Promise<void> {
  return invoke("copy_ocr_text", { id, hideWindow });
}

/**
 * Reads an image clip's dominant colours, extracting and storing them on the
 * first call so later visits are instant.
 */
export function ensureColorScheme(id: number): Promise<string[]> {
  return invoke<string[]>("ensure_color_scheme", { id });
}

/** Creates history entries from files dropped onto the window. */
export function importDroppedFiles(paths: string[]): Promise<number> {
  return invoke<number>("import_dropped_files", { paths });
}

/** Opens the settings page in its own decorated window. */
export function openSettingsWindow(): Promise<void> {
  return invoke("open_settings_window");
}

/** Opens the small window used to type a new clip. */
export function openAddClipWindow(): Promise<void> {
  return invoke("open_add_clip_window");
}

/**
 * Stores typed text as a new clip without touching the clipboard.
 * Returns the id of the inserted (or de-duplicated) clip.
 */
export function createTextClip(text: string): Promise<number> {
  return invoke<number>("create_text_clip", { text });
}

export function deleteClip(id: number): Promise<number> {
  return invoke<number>("delete_clip", { id });
}

export function deleteClips(ids: number[]): Promise<number> {
  return invoke<number>("delete_clips", { ids });
}

export function clearHistory(keepPinned = true): Promise<number> {
  return invoke<number>("clear_history", { keepPinned });
}

/** Applies the retention policy immediately instead of waiting for the next copy. */
export function pruneHistory(): Promise<number> {
  return invoke<number>("prune_history");
}

export function listCollections(): Promise<Collection[]> {
  return invoke<Collection[]>("list_collections");
}

export function createCollection(name: string, color?: string): Promise<Collection> {
  return invoke<Collection>("create_collection", { name, color });
}

export function renameCollection(id: number, name: string): Promise<void> {
  return invoke("rename_collection", { id, name });
}

export function deleteCollection(id: number): Promise<void> {
  return invoke("delete_collection", { id });
}

/** Moves a collection in or out of the archive set. */
export function setCollectionArchived(id: number, archived: boolean): Promise<void> {
  return invoke("set_collection_archived", { id, archived });
}

/** Opens Explorer at the clip's stored file, or at its original folder. */
export function revealClip(id: number): Promise<void> {
  return invoke("reveal_clip", { id });
}

/** Opens Explorer at an archive collection's dedicated folder. */
export function revealArchive(id: number): Promise<void> {
  return invoke("reveal_archive", { id });
}

export function storageInfo(): Promise<StorageInfo> {
  return invoke<StorageInfo>("storage_info");
}

/** Copies the history to `path` and restarts; never resolves on success. */
export function setStorageDir(path: string): Promise<void> {
  return invoke("set_storage_dir", { path });
}

export function setClipCollections(clipId: number, collectionIds: number[]): Promise<number[]> {
  return invoke<number[]>("set_clip_collections", { clipId, collectionIds });
}

/** Files a clip into exactly one archive, or `null` to bring it back out. */
export function setClipArchive(clipId: number, archiveId: number | null): Promise<number[]> {
  return invoke<number[]>("set_clip_archive", { clipId, archiveId });
}

/** Exports several clips into one folder. */
export function saveClips(ids: number[], dir: string): Promise<string[]> {
  return invoke<string[]>("save_clips", { ids, dir });
}

/** Writes text clips as Markdown notes; images and file clips are skipped. */
export function exportMarkdown(ids: number[], dir: string): Promise<MarkdownExport> {
  return invoke<MarkdownExport>("export_markdown", { ids, dir });
}

/** Puts arbitrary text on the clipboard (the emoji browser). */
export function copyText(text: string): Promise<void> {
  return invoke("copy_text", { text });
}

/** Copies an extracted scheme colour and records it as a colour clip. */
export function copyColor(hex: string): Promise<number> {
  return invoke<number>("copy_color", { hex });
}

/** Files a generated scheme into a new "Color Palette N" archive. */
export function saveColorPalette(colors: string[]): Promise<Collection> {
  return invoke<Collection>("save_color_palette", { colors });
}

export function listTags(): Promise<Tag[]> {
  return invoke<Tag[]>("list_tags");
}

export function setClipTags(clipId: number, tags: string[]): Promise<string[]> {
  return invoke<string[]>("set_clip_tags", { clipId, tags });
}

export function renameTag(id: number, name: string): Promise<void> {
  return invoke("rename_tag", { id, name });
}

export function deleteTag(id: number): Promise<void> {
  return invoke("delete_tag", { id });
}

export function setTagColor(id: number, color: string | null): Promise<void> {
  return invoke("set_tag_color", { id, color });
}

// ---- bulk actions ---------------------------------------------------------

export function setPinnedMany(ids: number[], pinned: boolean): Promise<number> {
  return invoke<number>("set_pinned_many", { ids, pinned });
}

export function addTagsMany(ids: number[], tags: string[]): Promise<number> {
  return invoke<number>("add_tags_many", { ids, tags });
}

export function addToCollection(ids: number[], collectionId: number): Promise<number> {
  return invoke<number>("add_to_collection", { ids, collectionId });
}

// ---- payloads -------------------------------------------------------------

/** Copies a clip's payload into `dir`, returning the files that were written. */
export function saveClip(id: number, dir: string): Promise<string[]> {
  return invoke<string[]>("save_clip", { id, dir });
}

/** Opens an http(s) link from a clip in the default browser. */
export function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

/** Opens the default mail client for an email clip. */
export function openEmail(email: string): Promise<void> {
  return invoke("open_email", { email });
}

/** Reveals any absolute path in Explorer. */
export function revealPath(path: string): Promise<void> {
  return invoke("reveal_path", { path });
}

/** Opens a path with its default application. */
export function openPath(path: string): Promise<void> {
  return invoke("open_path", { path });
}

/** Rich flavours captured with a text clip. */
export function getRichText(id: number): Promise<RichText> {
  return invoke<RichText>("get_rich_text", { id });
}

// ---- private mode ---------------------------------------------------------

export interface PauseState {
  paused: boolean;
  /** Unix-millisecond deadline of a timed pause. */
  until: number | null;
  /** True while paused "until restart". */
  indefinite: boolean;
}

export function captureState(): Promise<PauseState> {
  return invoke<PauseState>("capture_state");
}

export function pauseCapture(minutes?: number, untilRestart?: boolean): Promise<PauseState> {
  return invoke<PauseState>("pause_capture", { minutes, untilRestart });
}

export function resumeCapture(): Promise<PauseState> {
  return invoke<PauseState>("resume_capture");
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(patch: Partial<Settings>): Promise<Settings> {
  return invoke<Settings>("update_settings", { patch });
}

/** Reports whether a hotkey string parses, without saving it. */
export function validateHotkey(hotkey: string): Promise<string> {
  return invoke<string>("validate_hotkey", { hotkey });
}

export function hideWindow(): Promise<void> {
  return invoke("hide_window");
}

export function toggleWindow(): Promise<void> {
  return invoke("toggle_window");
}

export function revealDataDir(): Promise<void> {
  return invoke("reveal_data_dir");
}

/**
 * Builds a `media:` URL for a stored image file name.
 *
 * `convertFileSrc` produces the platform-correct form of the custom `media`
 * scheme registered in `lib.rs`; the backend handler accepts a bare file name.
 */
export function mediaUrl(fileName: string): string {
  return convertFileSrc(fileName, "media");
}

/** Normalises the string error Tauri hands back into something displayable. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}
