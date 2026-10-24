import { clipAction, type ClipAction } from "../lib/actions";
import { formatRelative, describeClip, clipBadgeClass, clipImageUrl, clipLabel, leafName } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { DragEvent as ReactDragEvent } from "react";
import type { ClipSummary } from "../lib/types";
import { ClipActionButton } from "./ClipActionButton";
import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch } from "./ColorSwatch";
import { SecretPreview } from "./SecretPreview";
import { TagChip } from "./TagChip";
import { tagColorOf } from "../lib/tagColor";
import {
  IconCopy,
  IconFiles,
  IconFolder,
  IconImage,
  IconNote,
  IconPin,
  IconText,
  IconTrash,
} from "./icons";

/** DOM id for a row, used by `aria-activedescendant` and scroll-into-view. */
export function clipRowId(id: number): string {
  return `clip-row-${id}`;
}

interface ClipListProps {
  items: ClipSummary[];
  selectedId: number | null;
  /** Ids picked with Ctrl/Shift for a bulk action. */
  selectedIds: number[];
  loading: boolean;
  emptyMessage: string;
  now: number;
  /** Tag name (lower-cased) to its stored colour, when it has one. */
  tagColors: ReadonlyMap<string, string | null>;
  onSelect: (id: number, modifiers: { ctrl: boolean; shift: boolean }) => void;
  onActivate: (clip: ClipSummary) => void;
  onCopy: (clip: ClipSummary) => void;
  onContextMenu: (clip: ClipSummary, event: ReactMouseEvent) => void;
  onTogglePin: (clip: ClipSummary) => void;
  onDelete: (clip: ClipSummary) => void;
  /** Runs a clip type's own action, such as opening a folder or a link. */
  onRunAction: (action: ClipAction) => void;
  /** Loads the payload so a drag can carry the full text. */
  onPrefetch: (clip: ClipSummary) => void;
  onDragStart: (clip: ClipSummary, event: ReactDragEvent) => void;
  /** False hides preview text, for archives configured to stay closed. */
  showContent?: boolean;
  /** False hides the note marker. */
  showNotes?: boolean;
}

function Thumbnail({ clip }: { clip: ClipSummary }) {
  if (clip.color) {
    return (
      <ColorSwatch
        color={clip.color}
        label={clip.color}
        showRgb
        className="h-9 w-12"
        textClassName="text-[7px]"
      />
    );
  }

  const image = clip.kind === "image" ? clipImageUrl(clip) : null;
  if (image) {
    return (
      <AnimatedImage
        still={clip.thumbFile ? mediaUrl(clip.thumbFile) : null}
        animated={clip.animated ? mediaUrl(`${clip.id}.gif`) : null}
        className="h-9 w-12 shrink-0 rounded border border-edge object-cover"
      />
    );
  }

  const Icon =
    clip.contentKind === "path" ? IconFolder : clip.kind === "files" ? IconFiles : IconText;
  return (
    <div className="flex h-9 w-12 shrink-0 items-center justify-center rounded border border-edge-soft bg-panel-3 text-ink-faint">
      <Icon className="size-4" />
    </div>
  );
}

function ClipRow({
  clip,
  slot,
  selected,
  multi,
  now,
  tagColors,
  onSelect,
  onActivate,
  onCopy,
  onContextMenu,
  onTogglePin,
  onDelete,
  onRunAction,
  onPrefetch,
  onDragStart,
  showContent,
  showNotes,
}: {
  clip: ClipSummary;
  /** Quick-paste number, 1–9, shown while the list is short enough to use them. */
  slot?: number;
  selected: boolean;
  multi: boolean;
  now: number;
  tagColors: ReadonlyMap<string, string | null>;
  onSelect: (modifiers: { ctrl: boolean; shift: boolean }) => void;
  onActivate: () => void;
  onCopy: () => void;
  onContextMenu: (event: ReactMouseEvent) => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onRunAction: (action: ClipAction) => void;
  onPrefetch: () => void;
  onDragStart: (event: ReactDragEvent) => void;
  showContent: boolean;
  showNotes: boolean;
}) {
  // The preview is a single collapsed line for text; file clips carry their names
  // in the preview already, so it reads naturally without special-casing.
  const primary = !showContent
    ? "Content hidden"
    : clip.kind === "image"
      ? clip.width && clip.height
        ? `${clip.width} × ${clip.height} image`
        : "Image"
      : clip.preview || "(empty)";
  const action = clipAction(clip);

  return (
    <div
      id={clipRowId(clip.id)}
      role="option"
      aria-selected={selected || multi}
      draggable
      onDragStart={onDragStart}
      onMouseEnter={onPrefetch}
      onMouseDown={(event) => {
        // Right-click must not disturb the selection: the context menu offers
        // actions for whatever is already picked.
        if (event.button !== 0) return;
        onSelect({ ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey });
      }}
      onDoubleClick={onActivate}
      onContextMenu={onContextMenu}
      className={`group relative flex cursor-default items-start gap-2.5 border-l-2 px-2.5 py-2 transition-colors ${
        selected || multi ? "border-accent bg-accent/10" : "border-transparent hover:bg-panel-2"
      }`}
    >
      <div className="relative mt-0.5 shrink-0">
        <Thumbnail clip={clip} />
        {slot !== undefined && (
          <span
            aria-hidden="true"
            className="absolute -bottom-1 -right-1 rounded border border-edge bg-panel-3 px-1 text-[9px] font-semibold tabular-nums text-ink shadow-sm"
          >
            {slot}
          </span>
        )}
      </div>

      <div className="min-w-0 flex-1">
        <p
          className={`truncate text-[12px] ${clip.pinned ? "text-ink" : "text-ink"}`}
          title={primary}
        >
          <SecretPreview level={clip.secretLevel} text={primary}>
            {primary}
          </SecretPreview>
        </p>

        <div className="mt-0.5 flex items-center gap-1.5 text-[10px] text-ink-faint">
          <span className={clipBadgeClass(clip)}>{clipLabel(clip)}</span>
          {showNotes && clip.hasNote && (
            <span title="Has a note" aria-label="Has a note" className="shrink-0 text-accent">
              <IconNote className="size-3" />
            </span>
          )}
          <span className="truncate">{describeClip(clip)}</span>
          {clip.sourceApp && (
            <>
              <span className="text-edge">·</span>
              <span className="truncate" title={clip.sourceApp}>
                {leafName(clip.sourceApp)}
              </span>
            </>
          )}
          <span className="text-edge">·</span>
          <span className="shrink-0 tabular-nums">{formatRelative(clip.updatedAt, now)}</span>
          {clip.copyCount > 1 && (
            <span className="shrink-0 tabular-nums">×{clip.copyCount}</span>
          )}
        </div>

        {showContent && clip.kind === "files" && clip.fileCount > 0 && (
          <p className="mt-0.5 truncate text-[10px] text-ink-faint" title={clip.preview}>
            {clip.preview.split(" · ").slice(1).join(" · ") || clip.preview}
          </p>
        )}

        {clip.tags.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {clip.tags.map((tag) => (
              <TagChip
                key={tag}
                name={tag}
                color={tagColorOf(tagColors, tag)}
                className="px-1 py-px text-[9px]"
              />
            ))}
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-0.5">
        {action && (
          <ClipActionButton
            action={action}
            onRun={onRunAction}
            className="rounded p-1 text-ink-faint opacity-0 transition-opacity hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
          />
        )}
        <button
          type="button"
          title="Copy without closing"
          aria-label="Copy clip"
          onClick={onCopy}
          className="rounded p-1 text-ink-faint opacity-0 transition-opacity hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
        >
          <IconCopy className="size-3.5" />
        </button>
        <button
          type="button"
          title={clip.pinned ? "Unpin" : "Pin to the top"}
          aria-label={clip.pinned ? "Unpin clip" : "Pin clip"}
          onClick={onTogglePin}
          className={`rounded p-1 transition-opacity ${
            clip.pinned
              ? "text-warn opacity-100"
              : "text-ink-faint opacity-0 hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
          }`}
        >
          <IconPin className="size-3.5" filled={clip.pinned} />
        </button>
        <button
          type="button"
          title="Delete (Del)"
          aria-label="Delete clip"
          onClick={onDelete}
          className="rounded p-1 text-ink-faint opacity-0 transition-opacity hover:text-danger group-hover:opacity-100 focus-visible:opacity-100"
        >
          <IconTrash className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

export function ClipList({
  items,
  selectedId,
  selectedIds,
  loading,
  emptyMessage,
  now,
  tagColors,
  onSelect,
  onActivate,
  onCopy,
  onContextMenu,
  onTogglePin,
  onDelete,
  onRunAction,
  onPrefetch,
  onDragStart,
  showContent = true,
  showNotes = true,
}: ClipListProps) {
  if (!loading && items.length === 0) {
    return (
      <div className="flex min-w-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center">
        <IconImage className="size-6 text-edge" />
        <p className="text-[12px] text-ink-dim">{emptyMessage}</p>
      </div>
    );
  }

  return (
    <div
      role="listbox"
      aria-label="Clipboard history"
      aria-activedescendant={selectedId ? clipRowId(selectedId) : undefined}
      className="min-w-0 flex-1 divide-y divide-edge-soft overflow-y-auto"
    >
      {items.map((clip, index) => (
        <ClipRow
          key={clip.id}
          clip={clip}
          slot={index < 9 ? index + 1 : undefined}
          selected={clip.id === selectedId}
          multi={selectedIds.includes(clip.id)}
          now={now}
          tagColors={tagColors}
          onSelect={(modifiers) => onSelect(clip.id, modifiers)}
          onActivate={() => onActivate(clip)}
          onCopy={() => onCopy(clip)}
          onContextMenu={(event) => onContextMenu(clip, event)}
          onTogglePin={() => onTogglePin(clip)}
          onDelete={() => onDelete(clip)}
          onRunAction={onRunAction}
          onPrefetch={() => onPrefetch(clip)}
          onDragStart={(event) => onDragStart(clip, event)}
          showContent={showContent}
          showNotes={showNotes}
        />
      ))}

      {loading && (
        <p className="px-3 py-4 text-center text-[11px] text-ink-faint">Loading…</p>
      )}
    </div>
  );
}

/** Keeps the highlighted row inside the viewport as the user arrows through it. */
export function scrollRowIntoView(id: number) {
  document.getElementById(clipRowId(id))?.scrollIntoView({ block: "nearest" });
}
