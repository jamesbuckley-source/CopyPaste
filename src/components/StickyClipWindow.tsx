import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch, displayHex, hslLabel, rgbLabel } from "./ColorSwatch";
import { IconClose, IconCopy, IconExternal, IconMinus, IconPlus } from "./icons";
import { clipFullImageUrl, clipLabel, clipMotionUrl } from "../lib/format";
import { renderMarkdown } from "../lib/markdown";
import { copyText, errorMessage, getClip, mediaUrl, openPath, openUrl } from "../lib/ipc";
import type { ClipDetail } from "../lib/types";
import { MaskedSecret } from "./SecretPreview";
import { useSettings } from "../hooks/useSettings";
import { applyTheme } from "../lib/theme";

/** A picture can be zoomed from a quarter size to four times, like the preview. */
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
/** One wheel step per burst, and a small step per click of the buttons. */
const WHEEL_INTERVAL_MS = 80;
const ZOOM_STEP = 0.25;
/** Quick enough to read as a zoom, slow enough to follow. */
const ZOOM_ANIMATION_MS = 140;

interface Point {
  x: number;
  y: number;
}

export function StickyClipWindow({ id }: { id: number | null }) {
  const [clip, setClip] = useState<ClipDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<Point>({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [animateZoom, setAnimateZoom] = useState(false);
  const [hovered, setHovered] = useState(false);
  /** Markdown opens rendered; the toggle keeps the source one click away. */
  const [rendered, setRendered] = useState(true);
  const { settings } = useSettings();
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ pointerId: number; origin: Point; pan: Point } | null>(null);
  /** When the last wheel step was taken, so a burst zooms once. */
  const wheelAtRef = useRef(0);

  useEffect(() => {
    if (!settings) return;
    applyTheme(settings.theme);
    document.documentElement.style.setProperty("--ui-scale", String(settings.textScale / 100));
  }, [settings]);

  useEffect(() => {
    let active = true;
    let request = 0;
    let unlisten: (() => void) | undefined;
    const refresh = () => {
      if (id === null) return;
      const currentRequest = ++request;
      getClip(id)
        .then((detail) => {
          if (active && currentRequest === request) {
            setClip(detail);
            setError(null);
          }
        })
        .catch((cause) => {
          if (active && currentRequest === request) setError(errorMessage(cause));
        });
    };
    refresh();
    listen("history:changed", refresh)
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch((cause) => {
        if (active) setError(errorMessage(cause));
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [id]);

  const close = useCallback(() => {
    void getCurrentWindow()
      .close()
      .catch((cause) => setError(errorMessage(cause)));
  }, []);

  const stepZoom = useCallback((direction: number) => {
    setAnimateZoom(!window.matchMedia("(prefers-reduced-motion: reduce)").matches);
    // Scaling is anchored on the centre, so a previous drag offset no longer holds.
    setPan({ x: 0, y: 0 });
    setZoom((current) => {
      const next = Math.round((current + direction * ZOOM_STEP) * 100) / 100;
      return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, next));
    });
  }, []);

  /** Keeps the picture's edges outside the viewport, so it can never be lost. */
  const clampPan = (next: Point): Point => {
    const viewport = viewportRef.current;
    const stage = stageRef.current;
    if (!viewport || !stage) return next;
    const maxX = Math.max(0, (stage.offsetWidth - viewport.clientWidth) / 2);
    const maxY = Math.max(0, (stage.offsetHeight - viewport.clientHeight) / 2);
    return {
      x: Math.min(maxX, Math.max(-maxX, next.x)),
      y: Math.min(maxY, Math.max(-maxY, next.y)),
    };
  };

  const startPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (zoom <= 1) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { pointerId: event.pointerId, origin: { x: event.clientX, y: event.clientY }, pan };
    setDragging(true);
  };

  const movePan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setPan(
      clampPan({
        x: drag.pan.x + (event.clientX - drag.origin.x),
        y: drag.pan.y + (event.clientY - drag.origin.y),
      }),
    );
  };

  const endPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [close]);

  useEffect(() => {
    const area = viewportRef.current;
    if (!area || !clip || clip.kind !== "image" || clip.video || clip.mediaLocked) return;
    // React's wheel handler is passive, so the listener is attached directly: the
    // page must not scroll under a picture that is being zoomed.
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      const now = performance.now();
      if (now - wheelAtRef.current < WHEEL_INTERVAL_MS) return;
      wheelAtRef.current = now;
      stepZoom(event.deltaY < 0 ? 1 : -1);
    };
    area.addEventListener("wheel", onWheel, { passive: false });
    return () => area.removeEventListener("wheel", onWheel);
  }, [clip, stepZoom]);

  const copy = (value: string) => {
    copyText(value)
      .then(() => {
        setNotice(`Copied ${value}`);
        window.setTimeout(() => setNotice(null), 1_500);
      })
      .catch((cause) => setError(errorMessage(cause)));
  };

  const text = clip?.text;
  const imageUrl = clip ? clipFullImageUrl(clip) : null;
  const motionUrl = clip ? clipMotionUrl(clip) : null;
  const isLink = clip?.contentKind === "url";
  const isMarkdown = clip?.contentKind === "markdown";
  const openLink = () => {
    const url = (text ?? clip?.preview ?? "").trim();
    if (url) openUrl(url).catch((cause) => setError(errorMessage(cause)));
  };

  return (
    <main className="flex h-full min-h-0 flex-col overflow-hidden border border-edge bg-panel text-ink">
      <header
        data-tauri-drag-region
        className="flex h-9 shrink-0 items-center gap-2 border-b border-edge bg-panel-2 px-2"
      >
        <span
          data-tauri-drag-region
          className="min-w-0 flex-1 truncate text-[11px] font-medium text-ink-dim"
        >
          {clip ? `Sticky ${clipLabel(clip)}` : "Sticky clip"}
        </span>
        {isLink && (
          <button
            type="button"
            onClick={openLink}
            title="Open this link in your browser"
            aria-label="Open this link"
            className="flex size-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-panel-3 hover:text-ink"
          >
            <IconExternal className="size-3.5" />
          </button>
        )}
        {isMarkdown && !clip?.secretLevel && (
          <button
            type="button"
            onClick={() => setRendered((current) => !current)}
            aria-pressed={rendered}
            title={rendered ? "Show the Markdown source" : "Show the formatting"}
            className="shrink-0 rounded px-1 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
          >
            {rendered ? "Plain" : "Markdown"}
          </button>
        )}
        {notice && (
          <span className="shrink-0 animate-pulse text-[10px] text-ok" role="status">
            {notice}
          </span>
        )}
        <button
          type="button"
          onClick={close}
          aria-label="Close sticky clip"
          title="Close"
          className="flex size-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-panel-3 hover:text-ink"
        >
          <IconClose className="size-3.5" />
        </button>
      </header>

      {error ? (
        <p role="alert" className="m-auto max-w-full p-4 text-xs text-danger">
          {error}
        </p>
      ) : id === null ? (
        <p role="alert" className="m-auto p-4 text-xs text-danger">
          This sticky clip link is invalid.
        </p>
      ) : !clip ? (
        <p className="m-auto p-4 text-xs text-ink-faint">Loading clip…</p>
      ) : clip.kind === "text" ? (
        clip.color && !clip.secretLevel ? (
          // The same panel the quick preview shows for a colour: the swatch, then
          // every notation with its own copy button.
          <div className="flex min-h-0 flex-1 gap-3 p-3">
            <ColorSwatch color={clip.color} className="w-1/3 self-stretch" textClassName="text-[11px]" />
            <div className="flex min-w-0 flex-1 flex-col justify-center gap-1.5">
              {[
                { label: "HEX", value: displayHex(clip.color) },
                { label: "RGB", value: rgbLabel(clip.color) },
                { label: "HSL", value: hslLabel(clip.color) },
              ].map((row) => (
                <div key={row.label} className="flex items-center gap-2">
                  <span className="w-8 shrink-0 text-[9px] font-semibold uppercase tracking-wider text-ink-faint">
                    {row.label}
                  </span>
                  <span data-selectable className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink">
                    {row.value}
                  </span>
                  <button
                    type="button"
                    onClick={() => copy(row.value)}
                    title={`Copy ${row.value}`}
                    aria-label={`Copy ${row.label} ${row.value}`}
                    className="flex shrink-0 items-center justify-center rounded border border-edge bg-panel px-1.5 py-1 text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                  >
                    <IconCopy className="size-3" />
                  </button>
                </div>
              ))}
            </div>
          </div>
        ) : (
          <div className="min-h-0 flex-1 overflow-auto p-3">
            {clip.secretLevel && (text == null || text === "") ? (
              <p className="whitespace-pre-wrap break-words font-mono text-xs text-ink-faint">
                Unlock the vault in CopyPaste to view this clip.
              </p>
            ) : isMarkdown && rendered && !clip.secretLevel ? (
              // Rendered into elements rather than HTML, so no clip can inject
              // markup into the window.
              <div data-selectable className="break-words rounded font-mono text-xs leading-relaxed text-ink-dim">
                {renderMarkdown(text ?? clip.preview)}
              </div>
            ) : (
              <pre
                // Text is opt-in selectable across the app, and a sticky note is
                // exactly the place someone wants to copy a line out of.
                data-selectable
                onDoubleClick={isLink ? openLink : undefined}
                title={isLink ? "Double-click to open the link" : undefined}
                className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-ink"
              >
                {clip.secretLevel && text != null ? (
                  <MaskedSecret text={text} />
                ) : (
                  text ?? clip.preview
                )}
              </pre>
            )}
          </div>
        )
      ) : clip.kind === "image" ? (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 items-center justify-center p-3">
            {clip.mediaLocked ? (
              <p className="text-xs text-ink-faint">
                Unlock the vault in CopyPaste to view this image.
              </p>
            ) : imageUrl ? (
              clip.video ? (
                <div
                  className="flex h-full w-full items-center justify-center"
                  onMouseEnter={() => setHovered(true)}
                  onMouseLeave={() => setHovered(false)}
                >
                  <video
                    src={motionUrl ?? imageUrl}
                    poster={clip.thumbFile ? mediaUrl(clip.thumbFile) : undefined}
                    // The transport controls would sit on top of the picture for as
                    // long as the window is up, so they appear while it is hovered.
                    controls={hovered}
                    autoPlay
                    loop
                    muted
                    playsInline
                    className="max-h-full max-w-full object-contain"
                  />
                </div>
              ) : (
                <div
                  ref={viewportRef}
                  onPointerDown={startPan}
                  onPointerMove={movePan}
                  onPointerUp={endPan}
                  onPointerCancel={endPan}
                  className={`relative h-full w-full touch-none select-none overflow-hidden ${
                    dragging ? "cursor-grabbing" : zoom > 1 ? "cursor-grab" : "cursor-default"
                  }`}
                >
                  <div
                    ref={stageRef}
                    style={{
                      width: `${zoom * 100}%`,
                      transform: `translate(-50%, -50%) translate(${pan.x}px, ${pan.y}px)`,
                      // Dragging has to track the pointer exactly, so the
                      // transition is only kept for button- and wheel-driven zoom.
                      transition:
                        dragging || !animateZoom
                          ? undefined
                          : `width ${ZOOM_ANIMATION_MS}ms ease-out, transform ${ZOOM_ANIMATION_MS}ms ease-out`,
                    }}
                    className="absolute left-1/2 top-1/2 checkerboard rounded"
                  >
                    <AnimatedImage
                      still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                      animated={motionUrl}
                      loop
                      alt="Sticky clipboard image"
                      loading="eager"
                      className="h-auto w-full"
                    />
                  </div>
                </div>
              )
            ) : (
              <p className="text-xs text-ink-faint">The image file is no longer available.</p>
            )}
          </div>

          {!clip.video && !clip.mediaLocked && imageUrl && (
            <div className="absolute bottom-2 right-2 flex items-center gap-0.5 rounded border border-edge bg-panel-2/95 px-1 py-0.5 shadow-md">
              <button
                type="button"
                onClick={() => stepZoom(-1)}
                disabled={zoom <= MIN_ZOOM}
                title="Zoom out"
                aria-label="Zoom out"
                className="flex items-center justify-center rounded p-0.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconMinus className="size-3" />
              </button>
              <button
                type="button"
                onClick={() => {
                  setAnimateZoom(!window.matchMedia("(prefers-reduced-motion: reduce)").matches);
                  setPan({ x: 0, y: 0 });
                  setZoom(1);
                }}
                title="Reset zoom"
                aria-label={`Reset zoom, currently ${Math.round(zoom * 100)}%`}
                className="min-w-11 px-0.5 text-[10px] tabular-nums text-ink-faint transition-colors hover:text-ink"
              >
                {Math.round(zoom * 100)}%
              </button>
              <button
                type="button"
                onClick={() => stepZoom(1)}
                disabled={zoom >= MAX_ZOOM}
                title="Zoom in"
                aria-label="Zoom in"
                className="flex items-center justify-center rounded p-0.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconPlus className="size-3" />
              </button>
            </div>
          )}
        </div>
      ) : (
        <ul className="min-h-0 flex-1 space-y-1 overflow-auto p-3 text-xs text-ink">
          {clip.filePaths.length > 0 ? (
            clip.filePaths.map((path, index) => (
              <li key={`${index}:${path}`} className="flex items-start gap-1.5">
                <span data-selectable className="min-w-0 flex-1 break-all font-mono">
                  {path}
                </span>
                <button
                  type="button"
                  onClick={() =>
                    openPath(path).catch((cause) => setError(errorMessage(cause)))
                  }
                  title={`Open ${path}`}
                  aria-label={`Open ${path}`}
                  className="mt-0.5 flex shrink-0 items-center justify-center rounded p-0.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                >
                  <IconExternal className="size-3" />
                </button>
              </li>
            ))
          ) : (
            <li className="text-ink-faint">No file paths are available.</li>
          )}
        </ul>
      )}
    </main>
  );
}
