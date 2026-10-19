/* Temporary benchmark harness: native emoji text glyphs vs a pre-rendered raster sheet. */
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";

import { ALL_EMOJI, type EmojiEntry } from "../lib/emoji";
import "../index.css";

const CELL = 38;
const GAP = 4;
const OVERSCAN = 4;
const GLYPH_CELL = 30;

type Mode = "text" | "raster" | "ascii" | "rasterblob" | "rasterwebp" | "textwarm";
type Handler = "raw" | "raf" | "row";

interface Sheet {
  url: string;
  cols: number;
  cell: number;
  bytes: number;
  ms?: number;
  drawMs?: number;
  encodeMs?: number;
}

interface RasterSheet extends Sheet {
  ms?: number;
  drawMs: number;
  encodeMs: number;
}

function drawSheet(): { canvas: HTMLCanvasElement; cols: number; cell: number; drawMs: number } {
  const dpr = window.devicePixelRatio || 1;
  const cols = 24;
  const rows = Math.ceil(ALL_EMOJI.length / cols);
  const cell = GLYPH_CELL;
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(cols * cell * dpr);
  canvas.height = Math.ceil(rows * cell * dpr);
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.font = `22px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  const started = performance.now();
  ALL_EMOJI.forEach((entry, index) => {
    const x = (index % cols) * cell + cell / 2;
    const y = Math.floor(index / cols) * cell + cell / 2;
    ctx.fillText(entry.char, x, y);
  });
  return { canvas, cols, cell, drawMs: performance.now() - started };
}

function buildSheet(): Sheet {
  const { canvas, cols, cell, drawMs } = drawSheet();
  return {
    url: canvas.toDataURL("image/png"),
    cols,
    cell,
    bytes: canvas.width * canvas.height * 4,
    ms: drawMs,
  };
}

async function buildRasterSheet(type: "image/png" | "image/webp" = "image/png"): Promise<RasterSheet> {
  const { canvas, cols, cell, drawMs } = drawSheet();
  const started = performance.now();
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, 0.95));
  const encodeMs = performance.now() - started;
  const url = URL.createObjectURL(blob!);
  const image = new Image();
  image.src = url;
  await image.decode();
  return {
    url,
    cols,
    cell,
    bytes: canvas.width * canvas.height * 4,
    drawMs,
    encodeMs,
  };
}

const Button = memo(function Button({
  entry,
  index,
  mode,
  sheet,
}: {
  entry: EmojiEntry;
  index: number;
  mode: Mode;
  sheet: Sheet | null;
}) {
  const glyph =
    mode === "ascii" ? (
      <span aria-hidden="true">A</span>
    ) : mode === "text" ? (
      <span aria-hidden="true">{entry.char}</span>
    ) : sheet ? (
      <span
        aria-hidden="true"
        style={{
          width: 22,
          height: 22,
          backgroundImage: `url(${sheet.url})`,
          backgroundPosition: `-${(index % sheet.cols) * sheet.cell + 4}px -${
            Math.floor(index / sheet.cols) * sheet.cell + 4
          }px`,
          backgroundSize: `${sheet.cols * sheet.cell}px auto`,
          backgroundRepeat: "no-repeat",
        }}
      />
    ) : (
      <span aria-hidden="true" />
    );

  return (
    <button
      type="button"
      title={`${entry.name} — click to copy`}
      aria-label={`${entry.name} emoji`}
      className="relative flex items-center justify-center rounded border border-transparent text-[22px] leading-none transition-colors hover:border-edge hover:bg-panel-3"
      style={{ height: CELL }}
    >
      {glyph}
    </button>
  );
});

function Bench({ mode, handler }: { mode: Mode; handler: Handler }) {
  const indexByChar = useMemo(() => {
    const map = new Map<string, number>();
    ALL_EMOJI.forEach((entry, index) => map.set(entry.char, index));
    return map;
  }, []);
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const latestScroll = useRef(0);
  const rafPending = useRef(false);
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const mountedAt = useRef(performance.now());

  useLayoutEffect(() => {
    if (mode !== "textwarm") return;
    const started = performance.now();
    drawSheet();
    const bench = (window as unknown as { __bench: Record<string, unknown> }).__bench;
    bench.warmMs = performance.now() - started;
  }, [mode]);

  useEffect(() => {
    if (mode === "raster") {
      setSheet(buildSheet());
    } else if (mode === "rasterblob" || mode === "rasterwebp") {
      const started = performance.now();
      buildRasterSheet(mode === "rasterwebp" ? "image/webp" : "image/png").then((ready) => {
        const bench = (window as unknown as { __bench: Record<string, unknown> }).__bench;
        bench.rasterTotalMs = performance.now() - started;
        bench.rasterDrawMs = ready.drawMs;
        bench.rasterEncodeMs = ready.encodeMs;
        setSheet(ready);
      });
    }
  }, [mode]);

  const onScroll = (element: HTMLElement) => {
    const next = element.scrollTop;
    latestScroll.current = next;
    if (handler === "raw") {
      setScrollTop(next);
      return;
    }
    if (handler === "row") {
      const row = Math.floor(next / (CELL + GAP));
      if (row !== Math.floor(scrollTop / (CELL + GAP))) setScrollTop(next);
      return;
    }
    if (rafPending.current) return;
    rafPending.current = true;
    requestAnimationFrame(() => {
      rafPending.current = false;
      setScrollTop(latestScroll.current);
    });
  };

  useEffect(() => {
    const element = scrollerRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      setViewport({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(element);
    setViewport({ width: element.clientWidth, height: element.clientHeight });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (viewport.height === 0) return;
    if ((mode === "raster" || mode === "rasterblob" || mode === "rasterwebp") && !sheet) return;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const bench = (window as unknown as { __bench: Record<string, unknown> }).__bench;
        bench.mountMs = performance.now() - mountedAt.current;
        bench.sheetMs = sheet?.ms ?? (mode === "rasterblob" || mode === "rasterwebp" ? Number(bench.rasterTotalMs ?? 0) : 0);
        bench.sheetBytes = sheet?.bytes ?? 0;
        bench.sheetUrlLength = sheet?.url.length ?? 0;
        bench.buttons = document.querySelectorAll("[data-bench-scroller] button").length;
        bench.ready = true;
      }),
    );
  }, [mode, sheet, viewport.height]);

  const columns = Math.max(1, Math.floor((viewport.width + GAP) / (CELL + GAP)));
  const emojiRows = useMemo(() => {
    const built: { entries: EmojiEntry[]; start: number }[] = [];
    for (let index = 0; index < ALL_EMOJI.length; index += columns) {
      built.push({ entries: ALL_EMOJI.slice(index, index + columns), start: index });
    }
    return built;
  }, [columns]);
  const totalHeight = emojiRows.length * (CELL + GAP);

  const visible = useMemo(() => {
    const first = Math.max(0, Math.floor(scrollTop / (CELL + GAP)) - OVERSCAN);
    const last = Math.min(
      emojiRows.length - 1,
      Math.floor((scrollTop + viewport.height) / (CELL + GAP)) + OVERSCAN,
    );
    const out: { row: (typeof emojiRows)[number]; top: number }[] = [];
    for (let index = first; index <= last; index += 1) {
      out.push({ row: emojiRows[index], top: index * (CELL + GAP) });
    }
    return out;
  }, [emojiRows, scrollTop, viewport.height]);

  return (
    <div data-bench-root className="flex h-full flex-col bg-canvas">
      <div
        ref={scrollerRef}
        data-bench-scroller
        onScroll={(event) => onScroll(event.currentTarget)}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      >
        <div style={{ height: totalHeight, position: "relative" }}>
          {visible.map(({ row, top }) => (
            <div
              key={row.start}
              style={{
                position: "absolute",
                top,
                left: 0,
                right: 0,
                height: CELL,
                display: "grid",
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                gap: `${GAP}px`,
              }}
            >
              {row.entries.map((entry, offset) => (
                <Button
                  key={`${entry.char}-${entry.name}`}
                  entry={entry}
                  index={indexByChar.get(entry.char) ?? row.start + offset}
                  mode={mode}
                  sheet={sheet}
                />
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

const params = new URLSearchParams(location.search);
const modeParam = params.get("mode");
const mode: Mode =
  modeParam === "raster" ||
  modeParam === "ascii" ||
  modeParam === "rasterblob" ||
  modeParam === "rasterwebp" ||
  modeParam === "textwarm"
    ? modeParam
    : "text";
const handler = (params.get("handler") as Handler | null) ?? "raw";

const bench = {
  ready: false,
  mountMs: 0,
  sheetMs: 0,
  sheetBytes: 0,
  sheetUrlLength: 0,
  buttons: 0,
  async encode() {
    const { canvas } = drawSheet();
    const out: Record<string, { ms: number; size: number }> = {};
    for (const type of ["image/png", "image/webp", "image/jpeg"]) {
      const started = performance.now();
      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, 0.9));
      const encodeMs = performance.now() - started;
      const decodeStarted = performance.now();
      const image = new Image();
      const url = URL.createObjectURL(blob!);
      image.src = url;
      await image.decode();
      const decodeMs = performance.now() - decodeStarted;
      URL.revokeObjectURL(url);
      out[type] = { ms: encodeMs, decodeMs, size: blob!.size } as unknown as { ms: number; size: number };
    }
    return out;
  },
  async scroll(step = 4, frames = 240) {
    const element = document.querySelector("[data-bench-scroller]") as HTMLElement;
    element.scrollTop = 0;
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const deltas: number[] = [];
    await new Promise<void>((resolve) => {
      let last = performance.now();
      let left = frames;
      const tick = (now: number) => {
        deltas.push(now - last);
        last = now;
        element.scrollTop += step;
        left -= 1;
        if (left > 0) requestAnimationFrame(tick);
        else resolve();
      };
      requestAnimationFrame(tick);
    });
    const sorted = [...deltas].sort((a, b) => a - b);
    return {
      frames: deltas.length,
      mean: deltas.reduce((sum, value) => sum + value, 0) / deltas.length,
      p50: sorted[Math.floor(sorted.length * 0.5)],
      p95: sorted[Math.floor(sorted.length * 0.95)],
      worst: sorted[sorted.length - 1],
      over20ms: deltas.filter((value) => value > 20).length,
      over33ms: deltas.filter((value) => value > 33).length,
    };
  },
};

(window as unknown as { __bench: typeof bench }).__bench = bench;

createRoot(document.getElementById("root")!).render(<Bench mode={mode} handler={handler} />);
