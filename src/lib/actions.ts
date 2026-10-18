import type { ClipSummary } from "./types";

/** What a clip type's inline action does; the caller routes it to the IPC wrapper. */
export type ClipActionKind = "open-url" | "reveal-path";

export interface ClipAction {
  kind: ClipActionKind;
  /** Button label, also used as the tooltip. */
  label: string;
  /** The URL or path the action works on, taken from the summary preview. */
  target: string;
  /** Used to re-read the full payload; previews are truncated for long paths. */
  clipId: number;
}

/**
 * The extra action a clip offers in the list and grid views, mirroring the
 * preview pane so the type can be acted on without opening it.
 */
export function clipAction(clip: ClipSummary): ClipAction | null {
  if (clip.kind !== "text") return null;
  const target = clip.preview.trim();
  if (!target || target.includes("\n")) return null;
  switch (clip.contentKind) {
    case "url":
      return { kind: "open-url", label: "Open link", target, clipId: clip.id };
    case "path":
      return { kind: "reveal-path", label: "Open Folder", target, clipId: clip.id };
    default:
      return null;
  }
}
