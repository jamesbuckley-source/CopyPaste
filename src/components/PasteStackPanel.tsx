/**
 * The paste stack popover: what is queued, in the order it will be pasted.
 *
 * It hangs off whichever button opened it, in the same fixed-position way the
 * context menu does. `#root` is scaled with CSS `zoom`, so the anchor has to be
 * divided by the root scale before it can position anything in here.
 */

import { useEffect, useRef } from "react";

import { clipImageUrl, clipLabel, clipMotionUrl } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { StackEntry } from "../lib/types";
import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch } from "./ColorSwatch";
import { IconClose, IconImage, IconStack, IconText } from "./icons";

/** The scale `#root` is drawn at, so measured pixels can be un-scaled. */
function rootScale(): number {
  const value = Number(
    getComputedStyle(document.documentElement).getPropertyValue("--ui-scale"),
  );
  return Number.isFinite(value) && value > 0 ? value : 1;
}

const PANEL_WIDTH = 300;
const EDGE = 8;

/** One queued item: a clip's thumbnail or a line from a split. */
function StackRow({
  entry,
  index,
  onRemove,
}: {
  entry: StackEntry;
  index: number;
  onRemove: () => void;
}) {
  const clip = entry.clip;
  const image = clip ? clipImageUrl(clip) : null;
  const label = clip ? clip.preview || clipLabel(clip) : (entry.text ?? "");

  return (
    <div className="group flex items-center gap-2 px-2 py-1.5 hover:bg-panel-2">
      <span className="w-4 shrink-0 text-right text-[10px] tabular-nums text-ink-faint">
        {index + 1}
      </span>
      {clip?.color ? (
        <ColorSwatch
          color={clip.color}
          label={clip.color}
          className="h-5 w-7"
          textClassName="hidden"
        />
      ) : clip && image ? (
        <AnimatedImage
          still={clip.thumbFile ? mediaUrl(clip.thumbFile) : null}
          animated={clip.video ? null : clipMotionUrl(clip)}
          video={clip.video ? clipMotionUrl(clip) : null}
          className="h-5 w-7 shrink-0 rounded border border-edge-soft object-cover"
        />
      ) : (
        <span className="flex h-5 w-7 shrink-0 items-center justify-center rounded border border-edge-soft bg-panel-3 text-ink-faint">
          {clip ? <IconImage className="size-2.5" /> : <IconText className="size-2.5" />}
        </span>
      )}
      <span className="min-w-0 flex-1 truncate text-[11px] text-ink" title={label}>
        {label || "(empty)"}
        {!clip && <span className="ml-1 text-[9px] uppercase text-ink-faint">line</span>}
      </span>
      <button
        type="button"
        onClick={onRemove}
        title="Remove from the stack"
        aria-label="Remove from the stack"
        className="shrink-0 rounded p-0.5 text-ink-faint opacity-0 transition-opacity hover:text-danger group-hover:opacity-100 focus-visible:opacity-100"
      >
        <IconClose className="size-3" />
      </button>
    </div>
  );
}

export function PasteStackPanel({
  anchor,
  items,
  shortcut,
  onClose,
  onPasteFirst,
  onRemove,
  onClear,
}: {
  /** Bottom-left corner of the button this hangs off, in viewport pixels. */
  anchor: { x: number; y: number };
  items: StackEntry[];
  /** The paste-next shortcut as the settings spell it, shown in the footer. */
  shortcut: string;
  onClose: () => void;
  /** Pastes the item at the top of the queue. */
  onPasteFirst: () => void;
  onRemove: (id: number) => void;
  onClear: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      if (!panelRef.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onClose();
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const scale = rootScale();
  const viewport = {
    width: window.innerWidth / scale,
    height: window.innerHeight / scale,
  };
  const left = Math.max(
    EDGE,
    Math.min(anchor.x / scale, viewport.width - PANEL_WIDTH - EDGE),
  );
  const top = Math.max(EDGE, Math.min(anchor.y / scale, viewport.height - 120));

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label="Paste stack"
      style={{ left, top, width: PANEL_WIDTH }}
      className="fixed z-50 flex max-h-[60vh] flex-col overflow-hidden rounded-panel border border-edge bg-panel shadow-xl animate-fade-in"
    >
      <div className="flex shrink-0 items-center gap-2 border-b border-edge-soft px-2 py-1.5">
        <IconStack className="size-3.5 shrink-0 text-accent" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-ink">
          Paste stack ({items.length})
        </span>
        <button
          type="button"
          onClick={onClear}
          title="Remove every queued item"
          className="shrink-0 rounded px-1.5 py-0.5 text-[10px] text-ink-faint transition-colors hover:bg-panel-3 hover:text-danger"
        >
          Clear all
        </button>
      </div>

      {items.length === 0 ? (
        <p className="px-3 py-4 text-[11px] leading-relaxed text-ink-faint">
          Nothing queued. Hover a clip and press the stack button, convert a collection,
          or split a text list.
        </p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {items.map((entry, index) => (
            <StackRow
              key={entry.id}
              entry={entry}
              index={index}
              onRemove={() => onRemove(entry.id)}
            />
          ))}
        </div>
      )}

      <div className="shrink-0 border-t border-edge-soft px-2 py-1.5">
        <button
          type="button"
          onClick={onPasteFirst}
          disabled={items.length === 0}
          className="mb-1 w-full rounded bg-accent/20 px-2 py-1 text-[11px] font-medium text-accent-hi transition-colors hover:bg-accent/30 disabled:opacity-50"
        >
          Paste next
        </button>
        <p className="text-[10px] leading-snug text-ink-faint">
          {shortcut ? `${shortcut} pastes the next item, in order` : "No shortcut set"}
          {" · each item is used up as it is pasted"}
        </p>
      </div>
    </div>
  );
}
