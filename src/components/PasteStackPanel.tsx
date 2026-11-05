/**
 * The paste stack, in two shapes: the popover that hangs off a button, and the
 * window it can be popped out into.
 *
 * The window version exists because the queue is useless if opening it hides the
 * application being pasted into. Both shapes read the same queue from the backend,
 * so popping out and docking back keeps the list exactly as it was.
 *
 * `#root` is scaled with CSS `zoom`, so the popover has to divide its anchor by the
 * root scale before positioning anything.
 */

import { useEffect, useRef, type ReactNode } from "react";

import { clipImageUrl, clipLabel, clipMotionUrl } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { StackEntry } from "../lib/types";
import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch } from "./ColorSwatch";
import { IconClose, IconExternal, IconImage, IconStack, IconText } from "./icons";

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

/** Numbered rows, the empty state, and the paste button with its shortcut. */
function StackBody({
  items,
  shortcut,
  onPasteFirst,
  onRemove,
  className,
}: {
  items: StackEntry[];
  shortcut: string;
  onPasteFirst: () => void;
  onRemove: (id: number) => void;
  /** The list's own sizing, which differs between the popover and the window. */
  className: string;
}) {
  return (
    <>
      {items.length === 0 ? (
        <p className={`px-3 py-4 text-[11px] leading-relaxed text-ink-faint ${className}`}>
          Nothing queued. Hover a clip and press the stack button, convert a collection,
          or split a text list.
        </p>
      ) : (
        <div className={`overflow-y-auto py-1 ${className}`}>
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
    </>
  );
}

/** Count, Clear all, and whichever window control this shape needs. */
function StackHeader({
  count,
  onClear,
  extra,
  draggable,
}: {
  count: number;
  onClear: () => void;
  /** Pop-out in the popover, close in the window. */
  extra: ReactNode;
  /** True in the window, where the header is how the window is moved. */
  draggable: boolean;
}) {
  // A frameless window moves by dragging, and only the element carrying this
  // attribute is a handle — so it goes on the header *and* its text, or the title
  // and the icon are dead spots. The popover must not carry it: there, dragging
  // the header would move the main window instead.
  const drag = draggable ? { "data-tauri-drag-region": true } : {};
  return (
    <div
      {...drag}
      className="flex shrink-0 items-center gap-1.5 border-b border-edge-soft px-2 py-1.5"
    >
      <span {...drag} className="shrink-0 text-accent">
        <IconStack className="size-3.5" />
      </span>
      <span {...drag} className="min-w-0 flex-1 truncate text-[11px] font-semibold text-ink">
        Paste stack ({count})
      </span>
      <button
        type="button"
        onClick={onClear}
        title="Remove every queued item"
        className="shrink-0 rounded px-1.5 py-0.5 text-[10px] text-ink-faint transition-colors hover:bg-panel-3 hover:text-danger"
      >
        Clear all
      </button>
      {extra}
    </div>
  );
}

const ICON_BUTTON_CLASS =
  "shrink-0 rounded p-1 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink";

/** The popover, anchored under whichever button opened it. */
export function PasteStackPanel({
  anchor,
  items,
  shortcut,
  onClose,
  onPopOut,
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
  /** Moves the queue into its own always-on-top window. */
  onPopOut: () => void;
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
      <StackHeader
        count={items.length}
        onClear={onClear}
        draggable={false}
        extra={
          <button
            type="button"
            onClick={onPopOut}
            title="Pop out into its own window"
            aria-label="Pop out into its own window"
            className={ICON_BUTTON_CLASS}
          >
            <IconExternal className="size-3" />
          </button>
        }
      />
      <StackBody
        items={items}
        shortcut={shortcut}
        onPasteFirst={onPasteFirst}
        onRemove={onRemove}
        className="min-h-0 flex-1"
      />
    </div>
  );
}

/**
 * The same queue filling its own small window, which the backend keeps above other
 * applications. Closing it closes it — the panel does not spring back open in the
 * main window; the queue is still there for the next time it is asked for.
 */
export function PasteStackWindow({
  items,
  shortcut,
  onClose,
  onPasteFirst,
  onRemove,
  onClear,
}: {
  items: StackEntry[];
  shortcut: string;
  onClose: () => void;
  onPasteFirst: () => void;
  onRemove: (id: number) => void;
  onClear: () => void;
}) {
  return (
    <div className="flex h-full flex-col overflow-hidden border border-edge bg-panel">
      <StackHeader
        count={items.length}
        onClear={onClear}
        draggable
        extra={
          <button
            type="button"
            onClick={onClose}
            title="Close this window"
            aria-label="Close this window"
            className={ICON_BUTTON_CLASS}
          >
            <IconClose className="size-3.5" />
          </button>
        }
      />
      <StackBody
        items={items}
        shortcut={shortcut}
        onPasteFirst={onPasteFirst}
        onRemove={onRemove}
        className="min-h-0 flex-1"
      />
    </div>
  );
}
