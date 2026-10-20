/**
 * Twemoji artwork for the emoji browser.
 *
 * Windows rasterises colour-emoji glyphs lazily, which made the first paint of
 * a category and every scroll into new rows stall. Each emoji is instead drawn
 * from a bundled SVG; the clipboard still receives the Unicode character.
 *
 * Twemoji graphics are CC BY 4.0 — see `public/emoji/twemoji/NOTICE.txt`.
 */

/** Bundled SVG for `char`, named after its code points with VS16 dropped. */
export function emojiImageUrl(char: string): string {
  const codePoints: string[] = [];
  for (const symbol of char) {
    const point = symbol.codePointAt(0);
    if (point !== undefined && point !== 0xfe0f) codePoints.push(point.toString(16));
  }
  return `/emoji/twemoji/${codePoints.join("-")}.svg`;
}
