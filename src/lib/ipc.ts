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
  GifQuality,
  GifResult,
  MarkdownExport,
  RecordStatus,
  RecordToolchain,
  RichText,
  Settings,
  StackEntry,
  SplitOutcome,
  Stats,
  VaultStatus,
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

/** The vault's state: "none", "locked" or "unlocked". */
export function vaultStatus(): Promise<VaultStatus> {
  return invoke<VaultStatus>("vault_status");
}

/** Creates the vault and leaves it open for this run. */
export function createVault(passphrase: string): Promise<void> {
  return invoke("create_vault", { passphrase });
}

export function unlockVault(passphrase: string): Promise<void> {
  return invoke("unlock_vault", { passphrase });
}

/** Forgets the key until it is unlocked again. Sealed clips stay sealed. */
export function lockVault(): Promise<void> {
  return invoke("lock_vault");
}

/** Rewraps the data key; the sealed clips themselves are not touched. */
export function changeVaultPassphrase(current: string, next: string): Promise<void> {
  return invoke("change_vault_passphrase", { current, next });
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

/** Hides the window and opens the onscreen ruler. */
export function openRuler(): Promise<void> {
  return invoke("open_ruler");
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

/** Shows the screen-recording toolbar, where a recording is started. */
export function openRecordBar(): Promise<void> {
  return invoke("open_record_bar");
}

/** The encoder and the audio devices it can see. */
export function recordToolchain(): Promise<RecordToolchain> {
  return invoke<RecordToolchain>("record_toolchain");
}

/** Installs FFmpeg with winget and answers with the last line of its output. */
export function installFfmpeg(): Promise<string> {
  return invoke<string>("install_ffmpeg");
}

/** What the recording is doing, for the timer and the button states. */
export function recordStatus(): Promise<RecordStatus> {
  return invoke<RecordStatus>("record_status");
}

/** Starts recording a region the user drags on screen. */
export function recordRegion(): Promise<void> {
  return invoke("record_region");
}

/** Starts recording the window the user clicks. */
export function recordWindow(): Promise<void> {
  return invoke("record_window");
}

/** Starts recording every monitor at once. */
export function recordScreen(): Promise<void> {
  return invoke("record_screen");
}

/** Opens the placement frame for a recording of a fixed size. */
export function recordFixed(width: number, height: number): Promise<void> {
  return invoke("record_fixed", { width, height });
}

/** Stops a screenshot countdown that is already running. */
export function cancelCaptureDelay(): Promise<void> {
  return invoke("cancel_capture_delay");
}

/** Records an error the window showed, so it is in the log as well as the banner. */
export function reportError(message: string): Promise<void> {
  return invoke("report_error", { message });
}

/** Shows the log file in Explorer. */
export function revealErrorLog(): Promise<void> {
  return invoke("reveal_error_log");
}

/** Writes a GIF converted from a video clip to `path`, and answers with it. */
export function exportGif(id: number, path: string): Promise<string> {
  return invoke("export_gif", { id, path });
}

/**
 * Converts a video clip into a GIF at `quality` and files it as a new clip,
 * answering with the new clip's id. `gif:progress` events report the way along.
 */
export function exportGifClip(
  id: number,
  quality: GifQuality,
): Promise<number> {
  return invoke("export_gif_clip", { id, quality });
}

/**
 * Starts the fixed-size recording once the frame has been placed.
 *
 * The rectangle is relative to the placement window, in CSS pixels.
 */
export function recordFixedArea(
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<void> {
  return invoke("record_fixed_area", { x, y, width, height });
}

export function recordPause(): Promise<void> {
  return invoke("record_pause");
}

export function recordResume(): Promise<void> {
  return invoke("record_resume");
}

/** Ends the recording; the backend answers once the file is saved. */
export function recordStop(): Promise<void> {
  return invoke("record_stop");
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

// ---- paste stack -----------------------------------------------------------

/** The queue, in paste order. */
export function stackList(): Promise<StackEntry[]> {
  return invoke<StackEntry[]>("stack_list");
}

/** Queues clips; resolves with how many were added, which is fewer at the cap. */
export function stackPush(ids: number[]): Promise<number> {
  return invoke<number>("stack_push", { ids });
}

/** Splits a text clip into one queued item per line. */
export function stackSplit(id: number): Promise<SplitOutcome> {
  return invoke<SplitOutcome>("stack_split", { id });
}

export function stackRemove(id: number): Promise<void> {
  return invoke("stack_remove", { id });
}

export function stackClear(): Promise<void> {
  return invoke("stack_clear");
}

/** Replaces the queue with a collection's clips, in view order. */
export function stackFromCollection(collectionId: number): Promise<number> {
  return invoke<number>("stack_from_collection", { collectionId });
}

/** Pastes the next queued item; resolves false when there was nothing queued. */
export function stackPasteNext(): Promise<boolean> {
  return invoke<boolean>("stack_paste_next");
}

/** Puts the text recognised in an image on the clipboard instead of the image. */
export function copyOcrText(id: number, hideWindow = true): Promise<void> {
  return invoke("copy_ocr_text", { id, hideWindow });
}

/** Puts only the clip's video file on the clipboard, without its address. */
export function copyVideoFile(id: number, hideWindow = true): Promise<void> {
  return invoke("copy_video_file", { id, hideWindow });
}

/** Puts the address the clip's video came from on the clipboard. */
export function copyVideoUrl(id: number, hideWindow = true): Promise<void> {
  return invoke("copy_video_url", { id, hideWindow });
}

/** Playing time of a clip's video in milliseconds, or null when it has none. */
export function clipLength(id: number): Promise<number | null> {
  return invoke<number | null>("clip_length", { id });
}

/**
 * Reads an image clip's dominant colours, extracting and storing them on the
 * first call so later visits are instant.
 */
export function ensureColorScheme(id: number): Promise<string[]> {
  return invoke<string[]>("ensure_color_scheme", { id });
}

/** A colour sampled from an image preview, and the scheme it now belongs to. */
export interface PickedColor {
  hex: string;
  /** False when the scheme already held the colour and was left alone. */
  added: boolean;
  colors: string[];
}

/**
 * Adds the colour under one point of an image to that clip's scheme.
 *
 * `x` and `y` are 0..1 across the image, so the size the preview is drawn at
 * never has to be known here.
 */
export function addSchemeColor(id: number, x: number, y: number): Promise<PickedColor> {
  return invoke<PickedColor>("add_scheme_color", { id, x, y });
}

/** Creates history entries from files dropped onto the window. */
export function importDroppedFiles(paths: string[]): Promise<number> {
  return invoke<number>("import_dropped_files", { paths });
}

/** Opens the settings page in its own decorated window. */
export function openSettingsWindow(): Promise<void> {
  return invoke("open_settings_window");
}

/** Opens settings scrolled to one section, by slug, e.g. "screen-recording". */
export function openSettingsSection(section: string): Promise<void> {
  return invoke("open_settings_section", { section });
}

/** Opens the small window used to type a new clip. */
export function openAddClipWindow(): Promise<void> {
  return invoke("open_add_clip_window");
}

/** Opens a stored clip in its own always-on-top window. */
export function openStickyClipWindow(id: number): Promise<void> {
  return invoke("open_sticky_clip_window", { id });
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

/** Opens an image clip in the external editor set in settings. */
export function editClip(id: number): Promise<void> {
  return invoke("edit_clip", { id });
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

/**
 * Keeps a clip in the hidden archive behind the GIF shelf, or releases it.
 *
 * Saved clips leave the history until they are taken off the shelf again.
 */
export function setClipSavedGif(clipId: number, saved: boolean): Promise<number[]> {
  return invoke<number[]>("set_clip_saved_gif", { clipId, saved });
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
/** Copies text the app produced — an emoji, a colour value — and records it. */
export function copyText(text: string, source?: string): Promise<void> {
  return invoke("copy_text", { text, source });
}

/** Copies an extracted scheme colour and records it as a colour clip. */
export function copyColor(hex: string): Promise<number> {
  return invoke<number>("copy_color", { hex });
}

/** Files a generated scheme into a new "Color Palette N" archive. */
export function saveColorPalette(colors: string[]): Promise<Collection> {
  return invoke<Collection>("save_color_palette", { colors });
}

/** Fetches the page's preview picture and stores the thumbnail on the clip. */
export function loadLinkPreview(id: number): Promise<string> {
  return invoke<string>("load_link_preview", { id });
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
