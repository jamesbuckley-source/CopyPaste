/**
 * Colours for a tag chip.
 *
 * The surface is opaque and built only from the tag's own colour, so the same
 * chip reads the same on the light and the dark theme. The ink is picked from
 * the surface's luminance — a pale tag gets dark text, a deep one white text —
 * rather than a fixed colour that would vanish on half the palette.
 *
 * Anything that is not a parseable hex colour (no colour, a legacy or imported
 * value) returns `null`, and the chip falls back to its neutral appearance.
 */

import type { Tag } from "./types";

export interface TagChipColors {
  background: string;
  border: string;
  text: string;
}

type Rgb = [number, number, number];

const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];

/** A shade off pure black, so 9 px chips are not harsh. */
const DARK_INK = "#101418";
const LIGHT_INK = "#ffffff";

/** WCAG luminance where black and white ink are equally readable. */
const INK_CUTOFF = 0.179;

const HEX_COLOR = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i;

function parseHexColor(color: string | null | undefined): Rgb | null {
  const match = color ? HEX_COLOR.exec(color.trim()) : null;
  if (!match) return null;
  const digits =
    match[1].length === 3
      ? match[1]
          .split("")
          .map((digit) => digit + digit)
          .join("")
      : match[1];
  return [
    Number.parseInt(digits.slice(0, 2), 16),
    Number.parseInt(digits.slice(2, 4), 16),
    Number.parseInt(digits.slice(4, 6), 16),
  ];
}

function linear(channel: number): number {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance: 0 for black, 1 for white. */
function luminance([red, green, blue]: Rgb): number {
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

function mix(from: Rgb, towards: Rgb, ratio: number): Rgb {
  return from.map((value, index) => Math.round(value + (towards[index] - value) * ratio)) as Rgb;
}

function toHex([red, green, blue]: Rgb): string {
  return `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
}

export function tagChipColors(color: string | null | undefined): TagChipColors | null {
  const rgb = parseHexColor(color);
  if (!rgb) return null;
  // Light colours fade toward white and deep colours darken, so the surface
  // always clears the cutoff for the ink it ends up with.
  const light = luminance(rgb) > INK_CUTOFF;
  const background = light ? mix(rgb, WHITE, 0.25) : mix(rgb, BLACK, 0.2);
  const border = light ? mix(rgb, BLACK, 0.15) : mix(rgb, WHITE, 0.3);
  return {
    background: toHex(background),
    border: toHex(border),
    text: luminance(background) > INK_CUTOFF ? DARK_INK : LIGHT_INK,
  };
}

/** Tag names are unique case-insensitively, so the lookup key is lower-cased. */
export function tagColorMap(tags: Tag[]): ReadonlyMap<string, string | null> {
  return new Map(tags.map((tag) => [tag.name.toLowerCase(), tag.color]));
}

/** Colour of the tag called `name`, matched case-insensitively. */
export function tagColorOf(
  colors: ReadonlyMap<string, string | null>,
  name: string,
): string | null {
  return colors.get(name.toLowerCase()) ?? null;
}
