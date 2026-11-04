/**
 * Placement overlay for a fixed-size recording.
 *
 * The bar used to hand the size to a drag picker that pinned the frame to the
 * pointer: every drag started somewhere else than where the frame already was,
 * and there was no way to start once it looked right. This window is the
 * replacement — it covers the desktop, the frame follows the pointer until the
 * first click places it, and the record button sits inside the frame so the size
 * on screen is exactly the size that gets recorded.
 *
 * Nothing here draws a picture: the frame is a bordered rectangle with a huge
 * spread shadow dimming everything around it, which reads the same over a dark
 * app and over a white desktop.
 */

import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { errorMessage, recordFixedArea } from "../lib/ipc";

/** Smallest frame the backend accepts, matching `record_fixed`. */
const MIN_SIZE = 64;

type Size = { width: number; height: number };
type Point = { x: number; y: number };

/** The frame size the window was opened for, taken from its URL. */
function requestedSize(): Size {
  const params = new URLSearchParams(window.location.search);
  const read = (key: string, fallback: number) => {
    const value = Number(params.get(key));
    return Number.isFinite(value) && value >= MIN_SIZE ? Math.round(value) : fallback;
  };
  return { width: read("width", 1280), height: read("height", 720) };
}

function clamp(value: number, low: number, high: number): number {
  // A frame larger than the screen has no room to move in: pin it to the edge
  // rather than pushing it off the desktop.
  return Math.min(Math.max(value, low), Math.max(low, high));
}

/** Keeps the frame's top-left corner inside the window. */
function clampBox(x: number, y: number, size: Size): Point {
  return {
    x: clamp(x, 0, window.innerWidth - size.width),
    y: clamp(y, 0, window.innerHeight - size.height),
  };
}

/** Top-left corner for a frame centred on the pointer. */
function centredOn(point: Point, size: Size): Point {
  return clampBox(point.x - size.width / 2, point.y - size.height / 2, size);
}

export function RecordArea() {
  const [size] = useState<Size>(requestedSize);
  // The sizes on offer are recording sizes — pixels of the finished video — so the
  // frame is drawn at their physical size rather than their CSS size, which is
  // what makes "1920 × 1080" mean the same thing on a 150% display.
  const ratio = window.devicePixelRatio || 1;
  const frameSize: Size = { width: size.width / ratio, height: size.height / ratio };
  /** `null` until the first click, which is what makes the frame follow the pointer. */
  const [frame, setFrame] = useState<Point | null>(null);
  const [pointer, setPointer] = useState<Point>(() => ({
    x: window.innerWidth / 2,
    y: window.innerHeight / 2,
  }));
  /** Offset from the frame's corner to the press, so a drag does not jump. */
  const [grab, setGrab] = useState<Point | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The app paints an opaque canvas behind its panels, which would turn this
  // window into a full-screen sheet of colour. It has to be see-through instead.
  useEffect(() => {
    const { body, documentElement } = document;
    const previous = [body.style.background, documentElement.style.background];
    body.style.background = "transparent";
    documentElement.style.background = "transparent";
    return () => {
      body.style.background = previous[0];
      documentElement.style.background = previous[1];
    };
  }, []);

  const close = useCallback(() => {
    void getCurrentWindow()
      .close()
      .catch(() => undefined);
  }, []);

  const start = useCallback(() => {
    if (busy) return;
    const box = frame ?? centredOn(pointer, frameSize);
    setBusy(true);
    setError(null);
    recordFixedArea(box.x, box.y, frameSize.width, frameSize.height).catch((e) => {
      setBusy(false);
      setError(errorMessage(e));
    });
  }, [busy, frame, pointer, frameSize.width, frameSize.height]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      } else if (event.key === "Enter") {
        event.preventDefault();
        start();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close, start]);

  const box = frame ?? centredOn(pointer, frameSize);

  const move = (event: React.MouseEvent) => {
    const point = { x: event.clientX, y: event.clientY };
    if (grab) {
      setFrame(clampBox(point.x - grab.x, point.y - grab.y, frameSize));
      return;
    }
    if (!frame) setPointer(point);
  };

  const press = (event: React.MouseEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const point = { x: event.clientX, y: event.clientY };
    // One gesture does both jobs: the first press places the frame under the
    // pointer, and any press also grabs it, so a click-and-drag places and moves
    // without the user having to click twice.
    const anchor = frame ?? centredOn(point, frameSize);
    setFrame(anchor);
    setGrab({ x: point.x - anchor.x, y: point.y - anchor.y });
  };

  const release = () => setGrab(null);

  return (
    <div
      className={`fixed inset-0 overflow-hidden ${frame ? "cursor-default" : "cursor-crosshair"}`}
      onMouseMove={move}
      onMouseDown={press}
      onMouseUp={release}
      onMouseLeave={release}
      onContextMenu={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <div
        className="absolute rounded-sm border-2 border-accent"
        style={{
          left: box.x,
          top: box.y,
          width: frameSize.width,
          height: frameSize.height,
          boxShadow: "0 0 0 9999px rgba(3, 7, 18, 0.45)",
        }}
      >
        <span className="absolute -top-7 left-0 whitespace-nowrap rounded bg-panel/90 px-2 py-0.5 text-[11px] font-medium tabular-nums text-ink shadow">
          {size.width} × {size.height}
        </span>
        <button
          type="button"
          onMouseDown={(event) => event.stopPropagation()}
          onClick={start}
          disabled={busy}
          title="Start recording this frame"
          className="absolute bottom-2 right-2 flex items-center gap-2 rounded-full bg-danger px-3 py-1.5 text-[12px] font-medium text-white shadow-lg transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          <span className="size-2.5 rounded-full bg-white" aria-hidden="true" />
          {busy ? "Starting…" : "Record"}
        </button>
      </div>

      <div className="pointer-events-none fixed left-1/2 top-4 flex -translate-x-1/2 flex-col items-center gap-1">
        <span className="whitespace-nowrap rounded-full bg-panel/90 px-3 py-1.5 text-[12px] text-ink-dim shadow-lg">
          Click to place · Drag to move · Enter to record · Esc to cancel
        </span>
        {error && (
          <span className="max-w-[420px] whitespace-normal rounded bg-panel/90 px-3 py-1 text-[11px] text-danger shadow-lg">
            {error}
          </span>
        )}
      </div>
    </div>
  );
}
