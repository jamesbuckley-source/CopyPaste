import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { clipFullImageUrl, describeClip } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { ClipDetail, ClipSummary } from "../lib/types";
import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch, displayHex } from "./ColorSwatch";
import { renderMarkdown } from "../lib/markdown";
import { MaskedSecret } from "./SecretPreview";
import { MarkdownToolbar } from "./MarkdownToolbar";
import { secretLabel } from "./SecretPreview";
import { IconClose, IconEyedropper, IconMinus, IconPlus, IconSave, IconShield } from "./icons";

interface ClipPreviewOverlayProps {
  clip: ClipSummary;
  /** Payload for `clip`; may arrive shortly after the overlay opens. */
  detail: ClipDetail | null;
  /** The image's colour scheme, or null while it is still being read. */
  schemeColors: string[] | null;
  schemeBusy: boolean;
  onCopyColor: (hex: string) => void;
  onSavePalette: (colors: string[]) => void;
  /** Adds the colour at a point given as a fraction of the image. */
  onPickColor: (x: number, y: number) => Promise<void>;
  onClose: () => void;
  onTextCommit: (clip: ClipSummary, text: string) => void;
}

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const ZOOM_STEP = 0.25;
/** Quick enough to feel immediate, slow enough to read as a zoom. */
const ZOOM_ANIMATION_MS = 150;
/** Wheel events arrive in bursts on a trackpad; one zoom step per window. */
const WHEEL_INTERVAL_MS = 80;
/** Quiet period before an edit reaches the store while typing continues. */
const SAVE_DELAY_MS = 400;

interface Point {
  x: number;
  y: number;
}

function clampZoom(value: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, value));
}

/**
 * Full-screen preview for the one selected clip. Space toggles it from the
 * history list, and the backdrop, Space or Escape close it.
 *
 * Text edits are written through `onTextCommit` a moment after typing stops and
 * again on blur or close, so the overlay never holds unsaved input.
 */
export function ClipPreviewOverlay({
  clip,
  detail,
  schemeColors,
  schemeBusy,
  onCopyColor,
  onSavePalette,
  onPickColor,
  onClose,
  onTextCommit,
}: ClipPreviewOverlayProps) {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<Point>({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  // Zoom transitions start at the first zoom change, never on the first paint.
  const [animateZoom, setAnimateZoom] = useState(false);
  const [secretShown, setSecretShown] = useState(false);
  const [editing, setEditing] = useState(false);
  /** Markdown clips open rendered; the toggle keeps the source one click away. */
  const [rendered, setRendered] = useState(true);
  /**
   * The size the user dragged the dialog to, or `null` for the opening size.
   *
   * Local state on purpose: the overlay unmounts on close, so every preview opens
   * at the default size again instead of reappearing at whatever the last drag
   * left behind.
   */
  const [size, setSize] = useState<PanelSize | null>(null);
  /** True while the eyedropper waits for a click on the image. */
  const [picking, setPicking] = useState(false);
  const [adding, setAdding] = useState(false);
  // A truncated preview is shown until `detail` loads, then replaced in place.
  const [draft, setDraft] = useState(() => detail?.text ?? clip.preview);

  const dialogRef = useRef<HTMLDivElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<HTMLTextAreaElement | null>(null);
  /** The pointer moving the image, with the offset it started from. */
  const dragRef = useRef<{ pointerId: number; origin: Point; pan: Point } | null>(null);
  /** When the last wheel step was taken, so a burst zooms once. */
  const wheelAtRef = useRef(0);
  const draftRef = useRef(draft);
  const savedRef = useRef(draft);
  /** True once the user has typed, so a late detail load cannot clobber it. */
  const touchedRef = useRef(false);
  const commitRef = useRef(onTextCommit);
  const clipRef = useRef(clip);

  useEffect(() => {
    commitRef.current = onTextCommit;
    clipRef.current = clip;
  });

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  const flush = useCallback(() => {
    const next = draftRef.current;
    if (!touchedRef.current || next === savedRef.current) return;
    // `set_clip_text` rejects an empty payload, which is what the detail pane's
    // editor enforces too: an emptied clip is simply not written.
    if (next.trim().length === 0) return;
    savedRef.current = next;
    touchedRef.current = false;
    commitRef.current(clipRef.current, next);
  }, []);

  // Typing is debounced; closing (which unmounts this component) flushes the
  // last keystrokes instead of dropping them.
  useEffect(() => {
    if (!editing || draft === savedRef.current) return;
    const timer = window.setTimeout(flush, SAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [draft, editing, flush]);

  useEffect(() => () => flush(), []);

  // The full payload usually arrives while the overlay is already open.
  useEffect(() => {
    const full = detail?.text;
    if (full === null || full === undefined || touchedRef.current) return;
    if (full === draftRef.current) return;
    draftRef.current = full;
    savedRef.current = full;
    setDraft(full);
  }, [detail?.text]);

  const close = () => {
    flush();
    onClose();
  };

  const changeDraft = (next: string) => {
    touchedRef.current = true;
    draftRef.current = next;
    setDraft(next);
  };

  /** Keeps the image's edges outside the viewport, so it can never be lost. */
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

  const changeZoom = (next: number) => {
    const value = clampZoom(next);
    if (value === zoom) return;
    // Scaling is anchored on the centre, so a drag offset no longer applies.
    setPan({ x: 0, y: 0 });
    setAnimateZoom(!window.matchMedia("(prefers-reduced-motion: reduce)").matches);
    setZoom(value);
  };

  /** One wheel step at a time, however many events a trackpad fires. */
  const stepZoom = (direction: number) => {
    const now = performance.now();
    if (now - wheelAtRef.current < WHEEL_INTERVAL_MS) return;
    wheelAtRef.current = now;
    changeZoom(zoom + direction * ZOOM_STEP);
  };

  // React's wheel handler is passive, so the listener is attached directly: the
  // page must not scroll underneath an image that is being zoomed.
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || clip.kind !== "image") return;
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      stepZoom(event.deltaY < 0 ? 1 : -1);
    };
    viewport.addEventListener("wheel", onWheel, { passive: false });
    return () => viewport.removeEventListener("wheel", onWheel);
  });

  /**
   * Samples the image where it was clicked, and hands the point over as a
   * fraction of the image so the zoom level never matters.
   */
  const pickColor = async (event: ReactPointerEvent<HTMLDivElement>) => {
    if (adding) return;
    const stage = stageRef.current;
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const x = (event.clientX - rect.left) / rect.width;
    const y = (event.clientY - rect.top) / rect.height;
    // One pick per armed click: the image goes back to panning afterwards.
    setPicking(false);
    setAdding(true);
    try {
      await onPickColor(x, y);
    } finally {
      setAdding(false);
    }
  };

  const startPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    // Stops the backdrop from reading the drag as a click to close, and stops
    // the browser from starting a native image drag or a text selection.
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { pointerId: event.pointerId, origin: { x: event.clientX, y: event.clientY }, pan };
    setDragging(true);
  };

  const movePan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setPan(
      clampPan({
        x: drag.pan.x + event.clientX - drag.origin.x,
        y: drag.pan.y + event.clientY - drag.origin.y,
      }),
    );
  };

  const endPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const secretBlurred = Boolean(clip.secretLevel) && !secretShown;
  // The list preview is truncated, so editing waits for the real payload just
  // as the detail pane's own editor does.
  const canEdit = detail !== null && !secretBlurred;
  const imageUrl = clipFullImageUrl(clip);
  const zoomPercent = Math.round(zoom * 100);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 p-6 backdrop-blur-sm animate-fade-in"
      onMouseDown={close}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`Preview of the selected ${clip.kind === "image" ? "image" : "text"} clip`}
        tabIndex={-1}
        onMouseDown={(event) => event.stopPropagation()}
        style={size ? { width: size.width, height: size.height } : undefined}
        className={`relative flex flex-col overflow-hidden rounded-panel border border-edge bg-panel shadow-xl animate-rise focus:outline-none ${
          size ? "" : "h-full max-h-[86vh] w-full max-w-[980px]"
        }`}
      >
        <div className="flex shrink-0 items-center gap-2 border-b border-edge-soft px-3 py-2">
          <h2 className="text-[11px] font-medium text-ink">
            {clip.kind === "image" ? "Image preview" : "Text preview"}
          </h2>
          <span className="min-w-0 truncate text-[10px] text-ink-faint">{describeClip(clip)}</span>
          <span className="flex-1" />
          <span className="hidden text-[10px] text-ink-faint sm:inline">
            {clip.kind === "image" ? "Scroll to zoom · Space or Esc to close" : "Space or Esc to close"}
          </span>
          <button
            type="button"
            onClick={close}
            title="Close the preview"
            aria-label="Close the preview"
            className="flex items-center justify-center rounded p-1 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
          >
            <IconClose className="size-3.5" />
          </button>
        </div>

        {clip.secretLevel && (
          <div className="flex shrink-0 items-center gap-1.5 border-b border-warn/30 bg-warn/10 px-3 py-1.5 text-[10px] leading-relaxed text-warn">
            <IconShield className="size-3 shrink-0" />
            <span className="min-w-0 flex-1">
              {secretLabel(clip.secretLevel)}. Hidden until you ask.
            </span>
            <button
              type="button"
              onClick={() => setSecretShown((shown) => !shown)}
              className="shrink-0 rounded border border-warn/40 px-1.5 py-px transition-colors hover:bg-warn/20"
            >
              {secretShown ? "Hide" : "Reveal"}
            </button>
          </div>
        )}

        {clip.kind === "image" ? (
          <>
            <div className="flex min-h-0 flex-1 p-3">
              {imageUrl ? (
                <div
                  ref={viewportRef}
                  onPointerDown={(event) => (picking ? void pickColor(event) : startPan(event))}
                  onPointerMove={movePan}
                  onPointerUp={endPan}
                  onPointerCancel={endPan}
                  className={`relative min-h-0 min-w-0 flex-1 touch-none select-none overflow-hidden rounded ${
                    picking ? "cursor-crosshair" : dragging ? "cursor-grabbing" : "cursor-grab"
                  }`}
                >
                  <div
                    ref={stageRef}
                    style={{
                      width: `${zoom * 100}%`,
                      transform: `translate(-50%, -50%) translate(${pan.x}px, ${pan.y}px)`,
                      // Dragging has to track the pointer exactly, so the
                      // transition is only kept for button-driven zooming.
                      transition:
                        dragging || !animateZoom
                          ? undefined
                          : `width ${ZOOM_ANIMATION_MS}ms ease-out, transform ${ZOOM_ANIMATION_MS}ms ease-out`,
                    }}
                    className={`absolute left-1/2 top-1/2 ${secretBlurred ? "blur-lg" : ""}`}
                  >
                    <AnimatedImage
                      still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                      animated={clip.animated ? mediaUrl(`${clip.id}.gif`) : null}
                      alt="Clipboard image"
                      loading="eager"
                      className="h-auto w-full rounded"
                    />
                  </div>
                </div>
              ) : (
                <p className="text-[11px] text-ink-faint">The image file is no longer available.</p>
              )}
            </div>

            {/* The scheme sits under the image so the swatches can be read
                against the picture, and scrolls sideways instead of growing
                taller, so the image keeps its room. */}
            <div className="flex shrink-0 items-center gap-2 border-t border-edge-soft bg-panel-2 px-3 py-1.5">
              <span className="shrink-0 text-[9px] font-semibold uppercase tracking-wider text-ink-faint">
                Colors
              </span>
              <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto py-0.5">
                {schemeColors === null || schemeColors.length === 0 ? (
                  <span className="truncate text-[10px] text-ink-faint">
                    {schemeBusy ? "Reading the colors…" : "No colors yet."}
                  </span>
                ) : (
                  schemeColors.map((hex) => (
                    <button
                      key={hex}
                      type="button"
                      onClick={() => onCopyColor(hex)}
                      title={`Copy ${displayHex(hex)}`}
                      className="shrink-0 rounded transition-transform hover:-translate-y-px"
                    >
                      <ColorSwatch
                        color={hex}
                        label={hex}
                        className="h-6 w-16"
                        textClassName="text-[8px]"
                      />
                    </button>
                  ))
                )}
              </div>
              <button
                type="button"
                onClick={() => setPicking((armed) => !armed)}
                disabled={adding}
                aria-pressed={picking}
                title={
                  picking
                    ? "Click the image to add that color · click here to cancel"
                    : "Add a color from the image"
                }
                aria-label="Add a color from the image"
                className={`flex shrink-0 items-center justify-center rounded border px-2 py-1 transition-colors disabled:opacity-40 ${
                  picking
                    ? "border-accent bg-accent/20 text-accent-hi"
                    : "border-edge bg-panel text-ink-dim hover:bg-panel-3 hover:text-ink"
                }`}
              >
                <IconEyedropper className="size-3.5" />
              </button>
              <button
                type="button"
                onClick={() => schemeColors && onSavePalette(schemeColors)}
                disabled={!schemeColors || schemeColors.length === 0}
                title="Save these colors into a new Color Palette archive"
                className="flex shrink-0 items-center gap-1 rounded border border-edge bg-panel px-2 py-1 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink disabled:opacity-40"
              >
                <IconSave className="size-3" />
                Save Color Palette
              </button>
            </div>

            <div className="flex shrink-0 items-center justify-center gap-1.5 border-t border-edge-soft bg-panel-2 px-3 py-2">
              <button
                type="button"
                onClick={() => changeZoom(zoom - ZOOM_STEP)}
                disabled={zoom <= MIN_ZOOM}
                title="Zoom out"
                aria-label="Zoom out"
                className="flex items-center justify-center rounded border border-edge bg-panel px-2 py-1 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconMinus className="size-3.5" />
              </button>
              <button
                type="button"
                onClick={() => changeZoom(1)}
                title="Reset zoom"
                aria-label={`Reset zoom, currently ${zoomPercent}%`}
                className="min-w-12 rounded border border-edge bg-panel px-2 py-1 text-[10px] tabular-nums text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
              >
                {zoomPercent}%
              </button>
              <button
                type="button"
                onClick={() => changeZoom(zoom + ZOOM_STEP)}
                disabled={zoom >= MAX_ZOOM}
                title="Zoom in"
                aria-label="Zoom in"
                className="flex items-center justify-center rounded border border-edge bg-panel px-2 py-1 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconPlus className="size-3.5" />
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="flex min-h-0 flex-1 flex-col p-3">
              {editing ? (
                <div className="flex min-h-0 flex-1 flex-col gap-2">
                  {clip.contentKind === "markdown" && (
                    <MarkdownToolbar
                      areaRef={editorRef}
                      onChange={changeDraft}
                      className="shrink-0 rounded border border-edge-soft bg-panel-2 px-1.5 py-1"
                    />
                  )}
                  <textarea
                    ref={editorRef}
                    autoFocus
                    value={draft}
                    onChange={(event) => changeDraft(event.target.value)}
                    onBlur={flush}
                    spellCheck={false}
                    aria-label="Edit clip text"
                    className="min-h-0 flex-1 resize-none rounded border border-accent-dim bg-panel-3 p-3 font-mono text-[13px] leading-relaxed text-ink focus:outline-none"
                  />
                </div>
              ) : clip.contentKind === "markdown" && rendered ? (
                // Rendered into elements rather than HTML, so no clip can inject
                // markup into the window.
                <div
                  data-selectable
                  className={`min-h-0 flex-1 overflow-auto break-words rounded bg-panel-2 p-3 text-[13px] leading-relaxed text-ink-dim ${
                    secretBlurred ? "blur-[5px] select-none" : ""
                  }`}
                >
                  {renderMarkdown(draft)}
                </div>
              ) : (
                <pre
                  data-selectable
                  className={`min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-3 font-mono text-[13px] leading-relaxed text-ink ${
                    secretBlurred ? "select-none" : ""
                  }`}
                >
                  {secretBlurred ? <MaskedSecret text={draft} /> : draft}
                </pre>
              )}
            </div>

            {/* Right padding leaves the corner free for the resize grip. */}
            <div className="flex shrink-0 items-center gap-2 border-t border-edge-soft bg-panel-2 py-2 pl-3 pr-7">
              <span className="min-w-0 flex-1 text-[10px] text-ink-faint">
                {editing
                  ? draft.trim().length === 0
                    ? "An empty clip cannot be saved."
                    : "Changes save automatically."
                    : "Edit to change the clip; changes save automatically."}
              </span>
              {editing ? (
                <button
                  type="button"
                  onClick={() => {
                    flush();
                    setEditing(false);
                  }}
                  className="shrink-0 rounded border border-accent/60 bg-accent/20 px-2.5 py-1 text-[11px] text-accent-hi transition-colors hover:bg-accent/30"
                >
                  Done
                </button>
              ) : (
                clip.contentKind === "markdown" && (
                  <button
                    type="button"
                    onClick={() => setRendered((current) => !current)}
                    aria-pressed={rendered}
                    title={rendered ? "Show the Markdown source" : "Show the formatting"}
                    className="shrink-0 rounded border border-edge bg-panel px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                  >
                    {rendered ? "Markdown" : "Plain"}
                  </button>
                )
              )}
              {!editing && (
                <button
                  type="button"
                  onClick={() => setEditing(true)}
                  disabled={!canEdit}
                  title={
                    detail === null
                      ? "Waiting for the clip to load"
                      : secretBlurred
                        ? "Reveal this clip before editing it"
                        : "Edit the clip text"
                  }
                  className="shrink-0 rounded border border-edge bg-panel px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink disabled:opacity-40"
                >
                  Edit
                </button>
              )}
            </div>
          </>
        )}
        <ResizeGrip size={size} onResize={setSize} />
      </div>
    </div>
  );
}

interface PanelSize {
  width: number;
  height: number;
}

/** Below this the dialog is a stamp, above it there is no room left to aim. */
const MIN_PANEL: PanelSize = { width: 420, height: 300 };

/**
 * The bottom-right grip that resizes the dialog.
 *
 * Rendered as a corner of the panel rather than as a native CSS `resize` handle,
 * because the native one is invisible in a webview until hovered and cannot be
 * clamped to a sensible minimum or to the space available.
 */
function ResizeGrip({
  size,
  onResize,
}: {
  size: PanelSize | null;
  onResize: (next: PanelSize) => void;
}) {
  const gripRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ pointerId: number; origin: { x: number; y: number }; from: PanelSize } | null>(
    null,
  );

  /** Where the dialog is right now, whether or not it has been dragged before. */
  const currentSize = (): PanelSize => {
    const panel = gripRef.current?.parentElement;
    if (!panel) return size ?? MIN_PANEL;
    const rect = panel.getBoundingClientRect();
    return { width: Math.round(rect.width), height: Math.round(rect.height) };
  };

  const start = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      origin: { x: event.clientX, y: event.clientY },
      from: currentSize(),
    };
  };

  const move = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    // The grips sit inside the viewport's padding, so the dialog can never be
    // dragged wider than the space it has, which would put the grip out of reach.
    const room = {
      width: window.innerWidth - 48,
      height: window.innerHeight - 48,
    };
    onResize({
      width: Math.min(Math.max(drag.from.width + event.clientX - drag.origin.x, MIN_PANEL.width), room.width),
      height: Math.min(
        Math.max(drag.from.height + event.clientY - drag.origin.y, MIN_PANEL.height),
        room.height,
      ),
    });
  };

  const end = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  return (
    <div
      ref={gripRef}
      role="separator"
      aria-label="Resize the preview"
      title="Drag to resize"
      onPointerDown={start}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
      className="absolute bottom-0 right-0 z-10 size-4 cursor-nwse-resize touch-none"
    >
      {/* Three short strokes, so the corner reads as a grip and not as a smudge. */}
      <svg viewBox="0 0 16 16" className="size-4 text-ink-faint" aria-hidden="true">
        <path d="M15 6 6 15M15 10.5 10.5 15" stroke="currentColor" strokeWidth="1.2" fill="none" />
      </svg>
    </div>
  );
}
