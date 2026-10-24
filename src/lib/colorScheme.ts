/** Builds the clipboard text for a list's colour scheme. */

import { displayHex } from "../components/ColorSwatch";
import type { ClipSummary } from "./types";

/**
 * Hex codes of the colour clips in `clips`, one per line, in list order so the
 * payload matches what the user sees. Repeats are dropped and each value is
 * upper-cased the way the UI shows hex. No trailing newline.
 */
export function colorSchemeText(clips: ClipSummary[]): string {
  // A Set keeps insertion order, so the first occurrence of each hex wins.
  const hexes = new Set<string>();
  for (const clip of clips) {
    if (clip.color) hexes.add(displayHex(clip.color));
  }
  return [...hexes].join("\n");
}
