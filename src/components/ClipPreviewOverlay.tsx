import { useCallback, useEffect, useRef, useState } from "react";
import { clipFullImageUrl, describeClip } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { ClipDetail, ClipSummary } from "../lib/types";
import { AnimatedImage } from "./AnimatedImage";
import { secretLabel } from "./SecretPreview";
import { IconClose, IconMinus, IconPlus, IconShield } from "./icons";

interface ClipPreviewOverlayProps {
  clip: ClipSummary;
  /** Payload for `clip`; may arrive shortly after the overlay opens. */
  detail: ClipDetail | null;
  onClose: () => void;
  onTextCommit: (clip: ClipSummary, text: string) => void;
}

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 4;
const ZOOM_STEP = 0.25;
/** Quiet period before an edit reaches the store while typing continues. */
const SAVE_DELAY_MS = 400;

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
  onClose,
  onTextCommit,
}: ClipPreviewOverlayProps) {
  const [zoom, setZoom] = useState(1);
  const [secretShown, setSecretShown] = useState(false);
  const [editing, setEditing] = useState(false);
  // A truncated preview is shown until `detail` loads, then replaced in place.
  const [draft, setDraft] = useState(() => detail?.text ?? clip.preview);

  const dialogRef = useRef<HTMLDivElement | null>(null);
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
        className="flex h-full max-h-[86vh] w-full max-w-[980px] flex-col overflow-hidden rounded-panel border border-edge bg-panel shadow-xl animate-rise focus:outline-none"
      >
        <div className="flex shrink-0 items-center gap-2 border-b border-edge-soft px-3 py-2">
          <h2 className="text-[11px] font-medium text-ink">
            {clip.kind === "image" ? "Image preview" : "Text preview"}
          </h2>
          <span className="min-w-0 truncate text-[10px] text-ink-faint">{describeClip(clip)}</span>
          <span className="flex-1" />
          <span className="hidden text-[10px] text-ink-faint sm:inline">Space or Esc to close</span>
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
            <div className="flex min-h-0 flex-1 overflow-auto p-3">
              {imageUrl ? (
                <div
                  style={{ width: `${zoom * 100}%` }}
                  className={`m-auto shrink-0 ${secretBlurred ? "blur-lg select-none" : ""}`}
                >
                  <AnimatedImage
                    still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                    animated={clip.animated ? mediaUrl(`${clip.id}.gif`) : null}
                    alt="Clipboard image"
                    loading="eager"
                    className="h-auto w-full rounded"
                  />
                </div>
              ) : (
                <p className="text-[11px] text-ink-faint">The image file is no longer available.</p>
              )}
            </div>

            <div className="flex shrink-0 items-center justify-center gap-1.5 border-t border-edge-soft bg-panel-2 px-3 py-2">
              <button
                type="button"
                onClick={() => setZoom((current) => clampZoom(current - ZOOM_STEP))}
                disabled={zoom <= MIN_ZOOM}
                title="Zoom out"
                aria-label="Zoom out"
                className="flex items-center justify-center rounded border border-edge bg-panel px-2 py-1 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconMinus className="size-3.5" />
              </button>
              <button
                type="button"
                onClick={() => setZoom(1)}
                title="Reset zoom"
                aria-label={`Reset zoom, currently ${zoomPercent}%`}
                className="min-w-12 rounded border border-edge bg-panel px-2 py-1 text-[10px] tabular-nums text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
              >
                {zoomPercent}%
              </button>
              <button
                type="button"
                onClick={() => setZoom((current) => clampZoom(current + ZOOM_STEP))}
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
                <textarea
                  autoFocus
                  value={draft}
                  onChange={(event) => changeDraft(event.target.value)}
                  onBlur={flush}
                  spellCheck={false}
                  aria-label="Edit clip text"
                  className="min-h-0 flex-1 resize-none rounded border border-accent-dim bg-panel-2 p-3 font-mono text-[13px] leading-relaxed text-ink focus:outline-none"
                />
              ) : (
                <pre
                  data-selectable
                  className={`min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-3 font-mono text-[13px] leading-relaxed text-ink ${
                    secretBlurred ? "blur-[5px] select-none" : ""
                  }`}
                >
                  {draft}
                </pre>
              )}
            </div>

            <div className="flex shrink-0 items-center gap-2 border-t border-edge-soft bg-panel-2 px-3 py-2">
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
      </div>
    </div>
  );
}
