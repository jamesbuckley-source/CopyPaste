import { formatRelative, KIND_LABEL } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { ClipSummary } from "../lib/types";
import { clipRowId } from "./ClipList";
import { IconCopy, IconFiles, IconImage, IconPin, IconText, IconTrash } from "./icons";

interface ClipGridProps {
  items: ClipSummary[];
  selectedId: number | null;
  loading: boolean;
  emptyMessage: string;
  now: number;
  /** Card size multiplier; 1 is the designed size. */
  scale: number;
  onSelect: (id: number) => void;
  onActivate: (clip: ClipSummary) => void;
  onCopy: (clip: ClipSummary) => void;
  /** Copies the text recognised in an image clip. */
  onCopyText: (clip: ClipSummary) => void;
  onContextMenu: (clip: ClipSummary, event: ReactMouseEvent) => void;
  onTogglePin: (clip: ClipSummary) => void;
  onDelete: (clip: ClipSummary) => void;
}

/** Card geometry derived from the user's scale setting. */
function geometry(scale: number) {
  const clamped = Math.min(2, Math.max(0.6, scale));
  return {
    minWidth: Math.round(148 * clamped),
    mediaHeight: Math.round(94 * clamped),
  };
}

function Card({
  clip,
  selected,
  now,
  scale,
  onSelect,
  onActivate,
  onCopy,
  onCopyText,
  onContextMenu,
  onTogglePin,
  onDelete,
}: {
  clip: ClipSummary;
  selected: boolean;
  now: number;
  scale: number;
  onSelect: () => void;
  onActivate: () => void;
  onCopy: () => void;
  onCopyText: () => void;
  onContextMenu: (event: ReactMouseEvent) => void;
  onTogglePin: () => void;
  onDelete: () => void;
}) {
  const { mediaHeight } = geometry(scale);
  const KindIcon = clip.kind === "files" ? IconFiles : clip.kind === "image" ? IconImage : IconText;

  return (
    <div
      id={clipRowId(clip.id)}
      role="option"
      aria-selected={selected}
      onMouseDown={onSelect}
      onDoubleClick={onActivate}
      onContextMenu={onContextMenu}
      className={`group relative flex cursor-default flex-col overflow-hidden rounded-lg border transition-colors ${
        selected
          ? "border-accent bg-accent/10"
          : "border-edge bg-panel-2 hover:border-edge-soft hover:bg-panel-3"
      }`}
    >
      <div
        className="relative shrink-0 overflow-hidden bg-canvas/70"
        style={{ height: mediaHeight }}
      >
        {clip.kind === "image" && clip.thumbFile ? (
          <img
            src={mediaUrl(clip.thumbFile)}
            alt=""
            loading="lazy"
            className="h-full w-full object-cover"
          />
        ) : (
          <p className="h-full w-full overflow-hidden whitespace-pre-wrap break-words p-2 text-[10px] leading-[1.45] text-ink-dim">
            {clip.preview || <span className="text-ink-faint">(empty)</span>}
          </p>
        )}

        {clip.kind !== "image" && (
          <KindIcon className="absolute bottom-1 right-1 size-3 text-ink-faint/50" />
        )}

        {clip.pinned && (
          <span className="absolute left-1 top-1 rounded bg-canvas/80 p-0.5 text-warn">
            <IconPin className="size-3" filled />
          </span>
        )}

        <div className="absolute right-1 top-1 flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <button
            type="button"
            title="Copy without closing"
            aria-label="Copy clip"
            onClick={(event) => {
              event.stopPropagation();
              onCopy();
            }}
            className="rounded bg-canvas/85 p-1 text-ink-dim hover:text-ink"
          >
            <IconCopy className="size-3" />
          </button>
          {clip.kind === "image" && clip.hasOcr && (
            <button
              type="button"
              title="Copy the text in this image"
              aria-label="Copy text from image"
              onClick={(event) => {
                event.stopPropagation();
                onCopyText();
              }}
              className="rounded bg-canvas/85 p-1 text-ink-dim hover:text-ink"
            >
              <IconText className="size-3" />
            </button>
          )}
          <button
            type="button"
            title={clip.pinned ? "Unpin" : "Pin to the top"}
            aria-label={clip.pinned ? "Unpin clip" : "Pin clip"}
            onClick={(event) => {
              event.stopPropagation();
              onTogglePin();
            }}
            className={`rounded bg-canvas/85 p-1 hover:text-ink ${
              clip.pinned ? "text-warn" : "text-ink-dim"
            }`}
          >
            <IconPin className="size-3" filled={clip.pinned} />
          </button>
          <button
            type="button"
            title="Delete (Del)"
            aria-label="Delete clip"
            onClick={(event) => {
              event.stopPropagation();
              onDelete();
            }}
            className="rounded bg-canvas/85 p-1 text-ink-dim hover:text-danger"
          >
            <IconTrash className="size-3" />
          </button>
        </div>
      </div>

      <div className="flex min-w-0 flex-col gap-1 p-2">
        <p className="truncate text-[11px] text-ink" title={clip.preview}>
          {clip.kind === "image"
            ? clip.width && clip.height
              ? `${clip.width} × ${clip.height}`
              : "Image"
            : clip.preview || "(empty)"}
        </p>

        <div className="flex items-center gap-1 text-[9px] text-ink-faint">
          <span className="shrink-0 rounded bg-panel-3 px-1 py-px">
            {KIND_LABEL[clip.kind]}
          </span>
          <span className="min-w-0 flex-1 truncate tabular-nums">
            {formatRelative(clip.updatedAt, now)}
          </span>
          {clip.copyCount > 1 && <span className="shrink-0 tabular-nums">×{clip.copyCount}</span>}
        </div>
      </div>
    </div>
  );
}

export function ClipGrid({
  items,
  selectedId,
  loading,
  emptyMessage,
  now,
  scale,
  onSelect,
  onActivate,
  onCopy,
  onCopyText,
  onContextMenu,
  onTogglePin,
  onDelete,
}: ClipGridProps) {
  if (!loading && items.length === 0) {
    return (
      <div className="flex min-w-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center">
        <IconImage className="size-6 text-edge" />
        <p className="text-[12px] text-ink-dim">{emptyMessage}</p>
      </div>
    );
  }

  const { minWidth } = geometry(scale);

  return (
    <div
      role="listbox"
      aria-label="Clipboard history"
      aria-activedescendant={selectedId ? clipRowId(selectedId) : undefined}
      className="min-w-0 flex-1 overflow-y-auto p-2"
    >
      <div
        className="grid gap-2"
        // Runtime value, so it cannot be a Tailwind class.
        style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${minWidth}px, 1fr))` }}
      >
        {items.map((clip) => (
          <Card
            key={clip.id}
            clip={clip}
            selected={clip.id === selectedId}
            now={now}
            scale={scale}
            onSelect={() => onSelect(clip.id)}
            onActivate={() => onActivate(clip)}
            onCopy={() => onCopy(clip)}
            onCopyText={() => onCopyText(clip)}
            onContextMenu={(event) => onContextMenu(clip, event)}
            onTogglePin={() => onTogglePin(clip)}
            onDelete={() => onDelete(clip)}
          />
        ))}
      </div>

      {loading && (
        <p className="px-3 py-4 text-center text-[11px] text-ink-faint">Loading…</p>
      )}
    </div>
  );
}
