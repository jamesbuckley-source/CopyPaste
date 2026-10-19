import type { EmojiEntry } from "./emoji";

/**
 * Emoji glyphs are rasterised lazily by the compositor, so a row that scrolls
 * into view for the first time can stall a frame or two. Drawing the glyphs
 * once into a scratch canvas fills Chromium's glyph cache, and the DOM text
 * that follows reuses it.
 *
 * The work is chunked through `requestIdleCallback` so opening the browser is
 * unaffected, and only glyphs that are not cached yet are drawn.
 */

const CHUNK = 4;
const TASK_BUDGET = 8;
const GLYPH_SIZE = 22;
const CELL = 30;

let warmedKey = "";
const warmed = new Set<string>();

function rasterScale(): { dpr: number; uiScale: number } {
  const dpr = window.devicePixelRatio || 1;
  const configured = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--ui-scale"),
  );
  return { dpr, uiScale: Number.isFinite(configured) && configured > 0 ? configured : 1 };
}

function flush(canvas: HTMLCanvasElement, context: CanvasRenderingContext2D) {
  // The canvas draw is deferred until something reads it back. Reading a small
  // scratch canvas forces the glyph rasterisation that the first paint of the
  // corresponding DOM text would otherwise trigger mid-scroll.
  context.getImageData(0, 0, canvas.width, canvas.height);
}

/** Warms the glyph cache for `entries`; returns a function that stops the work. */
export function warmEmojiGlyphs(entries: readonly EmojiEntry[]): () => void {
  if (typeof window === "undefined" || typeof requestIdleCallback === "undefined") {
    return () => {};
  }

  const { dpr, uiScale } = rasterScale();
  const key = `${GLYPH_SIZE}@${dpr * uiScale}`;
  if (key !== warmedKey) {
    warmedKey = key;
    warmed.clear();
  }

  const pending = entries.filter((entry) => !warmed.has(entry.char));
  if (pending.length === 0) return () => {};

  const canvas = document.createElement("canvas");
  const cell = CELL * uiScale;
  canvas.width = Math.ceil(CHUNK * cell * dpr);
  canvas.height = Math.ceil(cell * dpr);
  const context = canvas.getContext("2d", { alpha: true });
  if (!context) return () => {};
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.font = `${GLYPH_SIZE * uiScale}px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif`;
  context.textAlign = "center";
  context.textBaseline = "middle";

  let cancelled = false;
  let index = 0;
  let handle = 0;

  const warm = (deadline: IdleDeadline) => {
    if (cancelled) return;
    const started = performance.now();
    let drew = false;
    while (index < pending.length) {
      // Always draw one chunk, then keep drawing only while both the idle
      // deadline and our own small budget allow it. A timed-out callback must
      // not drain the whole queue in a single long task.
      if (drew && (performance.now() - started > TASK_BUDGET || deadline.timeRemaining() <= 0)) {
        break;
      }
      const count = Math.min(CHUNK, pending.length - index);
      context.clearRect(0, 0, CHUNK * cell, cell);
      for (let offset = 0; offset < count; offset += 1) {
        context.fillText(pending[index + offset].char, offset * cell + cell / 2, cell / 2);
      }
      flush(canvas, context);
      for (let offset = 0; offset < count; offset += 1) warmed.add(pending[index + offset].char);
      index += count;
      drew = true;
    }
    if (index < pending.length) handle = requestIdleCallback(warm);
  };

  handle = requestIdleCallback(warm, { timeout: 500 });
  return () => {
    cancelled = true;
    if (handle) cancelIdleCallback(handle);
  };
}
