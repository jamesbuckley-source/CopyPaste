/** Presentation helpers shared by the list, detail pane and settings. */

import type { ClipKind, ClipSummary } from "./types";

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** "just now" / "12m ago" / "3h ago" / "5d ago" / a date beyond a week. */
export function formatRelative(timestampMs: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - timestampMs) / 1000));
  if (seconds < 45) return "just now";
  if (seconds < 3_600) return `${Math.round(seconds / 60)}m ago`;
  if (seconds < 86_400) return `${Math.round(seconds / 3_600)}h ago`;
  if (seconds < 7 * 86_400) return `${Math.round(seconds / 86_400)}d ago`;
  return new Date(timestampMs).toLocaleDateString();
}

export function formatCount(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export const KIND_LABEL: Record<ClipKind, string> = {
  text: "Text",
  image: "Image",
  files: "Files",
};

/** One-line description of a clip's shape, shown under the preview. */
export function describeClip(clip: ClipSummary): string {
  switch (clip.kind) {
    case "text":
      return `${formatCount(clip.charCount, "character")} · ${formatBytes(clip.byteSize)}`;
    case "image": {
      const dimensions =
        clip.width && clip.height ? `${clip.width} × ${clip.height}` : "image";
      return `${dimensions} · ${formatBytes(clip.byteSize)}`;
    }
    case "files":
      return `${formatCount(clip.fileCount, "file")} · ${formatBytes(clip.byteSize)}`;
  }
}

/** Strips the directory from a Windows or POSIX path. */
export function leafName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** Truncates from the middle so both ends of a long path stay readable. */
export function shortenMiddle(value: string, maxLength = 64): string {
  if (value.length <= maxLength) return value;
  const half = Math.floor((maxLength - 1) / 2);
  return `${value.slice(0, half)}…${value.slice(value.length - half)}`;
}
