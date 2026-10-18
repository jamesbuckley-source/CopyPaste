/**
 * Theme palettes and the bridge to CSS.
 *
 * The palette lives in the backend settings so it travels with the rest of the
 * configuration, but the interface only ever reads CSS custom properties. Applying
 * a theme is therefore a handful of `setProperty` calls, and every Tailwind color
 * utility follows along because they compile to `var(--color-*)`.
 */

import type { Theme, ThemeColors } from "./types";

export const DARK_COLORS: ThemeColors = {
  canvas: "#0b0e14",
  panel: "#12161f",
  panel2: "#181d29",
  panel3: "#1f2533",
  edge: "#252c3b",
  edgeSoft: "#1c2230",
  ink: "#e7eaf0",
  inkDim: "#99a3b5",
  inkFaint: "#6a7488",
  accent: "#6d8cff",
  accentHi: "#8aa2ff",
  danger: "#ff6b6b",
};

export const LIGHT_COLORS: ThemeColors = {
  canvas: "#eef1f6",
  panel: "#ffffff",
  panel2: "#f5f7fb",
  panel3: "#e6eaf2",
  edge: "#d2d8e4",
  edgeSoft: "#e6eaf2",
  ink: "#1b2130",
  inkDim: "#525d75",
  inkFaint: "#8590a6",
  accent: "#3559d6",
  accentHi: "#2748b8",
  danger: "#cf3b3b",
};

/** Editable palette entries, in the order the settings page lists them. */
export const COLOR_FIELDS: { key: keyof ThemeColors; label: string; hint: string }[] = [
  { key: "canvas", label: "Background", hint: "The window behind everything else." },
  { key: "panel", label: "Surface", hint: "Sidebar, title bar and settings background." },
  { key: "panel2", label: "Raised surface", hint: "Inputs, cards and hover states." },
  { key: "panel3", label: "Sunken surface", hint: "Chips, badges and pressed states." },
  { key: "edge", label: "Border", hint: "Dividers and control outlines." },
  { key: "edgeSoft", label: "Soft border", hint: "Quieter separators inside lists." },
  { key: "ink", label: "Text", hint: "Primary text color." },
  { key: "inkDim", label: "Muted text", hint: "Labels and secondary information." },
  { key: "inkFaint", label: "Faint text", hint: "Hints, placeholders and metadata." },
  { key: "accent", label: "Accent", hint: "Selection, focus and active states." },
  { key: "accentHi", label: "Accent highlight", hint: "Accent text on tinted backgrounds." },
  { key: "danger", label: "Danger", hint: "Delete actions and error banners." },
];

export function resolveColors(theme: Theme): ThemeColors {
  if (theme.mode === "light") return LIGHT_COLORS;
  if (theme.mode === "custom") return theme.colors;
  return DARK_COLORS;
}

function parseHex(hex: string): [number, number, number] {
  const value = hex.replace("#", "");
  return [
    Number.parseInt(value.slice(0, 2), 16) || 0,
    Number.parseInt(value.slice(2, 4), 16) || 0,
    Number.parseInt(value.slice(4, 6), 16) || 0,
  ];
}

function toHex(channels: [number, number, number]): string {
  return `#${channels
    .map((channel) => Math.round(Math.min(255, Math.max(0, channel))).toString(16).padStart(2, "0"))
    .join("")}`;
}

/** Blends `towards` into `base`; used for the few tones the palette omits. */
function mix(base: string, towards: string, ratio: number): string {
  const a = parseHex(base);
  const b = parseHex(towards);
  return toHex([
    a[0] + (b[0] - a[0]) * ratio,
    a[1] + (b[1] - a[1]) * ratio,
    a[2] + (b[2] - a[2]) * ratio,
  ]);
}

/** Writes the palette onto the document so the whole interface follows it. */
export function applyTheme(theme: Theme) {
  const colors = resolveColors(theme);
  const root = document.documentElement.style;

  root.setProperty("--color-canvas", colors.canvas);
  root.setProperty("--color-panel", colors.panel);
  root.setProperty("--color-panel-2", colors.panel2);
  root.setProperty("--color-panel-3", colors.panel3);
  root.setProperty("--color-edge", colors.edge);
  root.setProperty("--color-edge-soft", colors.edgeSoft);
  root.setProperty("--color-ink", colors.ink);
  root.setProperty("--color-ink-dim", colors.inkDim);
  root.setProperty("--color-ink-faint", colors.inkFaint);
  root.setProperty("--color-accent", colors.accent);
  root.setProperty("--color-accent-hi", colors.accentHi);
  root.setProperty("--color-danger", colors.danger);
  // Focus rings need a tone between the accent and the surface behind it.
  root.setProperty("--color-accent-dim", mix(colors.canvas, colors.accent, 0.28));
  root.setProperty("--scrollbar-thumb", mix(colors.edge, colors.inkFaint, 0.25));
  root.setProperty("--scrollbar-thumb-hover", mix(colors.edge, colors.inkFaint, 0.6));
}
