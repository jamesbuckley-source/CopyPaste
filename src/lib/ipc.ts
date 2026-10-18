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
  Settings,
  Stats,
  Tag,
} from "./types";

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
 * this one action.
 */
export function copyClip(id: number, autoPaste?: boolean): Promise<void> {
  return invoke("copy_clip", { id, autoPaste });
}

export function setPinned(id: number, pinned: boolean): Promise<void> {
  return invoke("set_pinned", { id, pinned });
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

export function setClipCollections(clipId: number, collectionIds: number[]): Promise<number[]> {
  return invoke<number[]>("set_clip_collections", { clipId, collectionIds });
}

export function listTags(): Promise<Tag[]> {
  return invoke<Tag[]>("list_tags");
}

export function setClipTags(clipId: number, tags: string[]): Promise<string[]> {
  return invoke<string[]>("set_clip_tags", { clipId, tags });
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
