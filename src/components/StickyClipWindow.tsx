import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { AnimatedImage } from "./AnimatedImage";
import { IconClose, IconExternal, IconMinus, IconPlus } from "./icons";
import { clipFullImageUrl, clipLabel, clipMotionUrl } from "../lib/format";
import { errorMessage, getClip, mediaUrl, openUrl } from "../lib/ipc";
import type { ClipDetail } from "../lib/types";
import { MaskedSecret } from "./SecretPreview";
import { useSettings } from "../hooks/useSettings";
import { applyTheme } from "../lib/theme";

/** A picture can be zoomed from a quarter size to four times, like the preview. */
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
/** One step per wheel burst, and a tenth per click of the buttons. */
const WHEEL_INTERVAL_MS = 80;
const ZOOM_STEP = 1.15;

export function StickyClipWindow({ id }: { id: number | null }) {
  const [clip, setClip] = useState<ClipDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [hovered, setHovered] = useState(false);
  const { settings } = useSettings();
  const mediaRef = useRef<HTMLDivElement | null>(null);
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

  const stepZoom = useCallback((factor: number) => {
    setZoom((current) => {
      const next = current * factor;
      return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(next * 100) / 100));
    });
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [close]);

  useEffect(() => {
    const area = mediaRef.current;
    if (!area || !clip || clip.kind !== "image" || clip.video || clip.mediaLocked) return;
    // React's wheel handler is passive, so the listener is attached directly: the
    // page must not scroll under a picture that is being zoomed.
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      const now = performance.now();
      if (now - wheelAtRef.current < WHEEL_INTERVAL_MS) return;
      wheelAtRef.current = now;
      stepZoom(event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP);
    };
    area.addEventListener("wheel", onWheel, { passive: false });
    return () => area.removeEventListener("wheel", onWheel);
  }, [clip, stepZoom]);

  const text = clip?.text;
  const imageUrl = clip ? clipFullImageUrl(clip) : null;
  const motionUrl = clip ? clipMotionUrl(clip) : null;
  const isLink = clip?.contentKind === "url";
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
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {clip.secretLevel && (text == null || text === "") ? (
            <p className="whitespace-pre-wrap break-words font-mono text-xs text-ink-faint">
              Unlock the vault in CopyPaste to view this clip.
            </p>
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
      ) : clip.kind === "image" ? (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div
            ref={mediaRef}
            className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-3"
          >
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
                <div className="mx-auto w-full" style={{ width: `${zoom * 100}%` }}>
                  <AnimatedImage
                    still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                    animated={motionUrl}
                    loop
                    alt="Sticky clipboard image"
                    loading="eager"
                    className="h-auto w-full object-contain"
                  />
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
                onClick={() => stepZoom(1 / ZOOM_STEP)}
                disabled={zoom <= MIN_ZOOM}
                title="Zoom out"
                aria-label="Zoom out"
                className="flex items-center justify-center rounded p-0.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconMinus className="size-3" />
              </button>
              <button
                type="button"
                onClick={() => setZoom(1)}
                title="Reset zoom"
                aria-label={`Reset zoom, currently ${Math.round(zoom * 100)}%`}
                className="min-w-11 px-0.5 text-[10px] tabular-nums text-ink-faint transition-colors hover:text-ink"
              >
                {Math.round(zoom * 100)}%
              </button>
              <button
                type="button"
                onClick={() => stepZoom(ZOOM_STEP)}
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
              <li key={`${index}:${path}`} className="break-all font-mono">
                {path}
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
