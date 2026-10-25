import { clipAction, type ClipAction } from "../lib/actions";
import { formatRelative, clipBadgeClass, clipImageUrl, clipLabel } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { DragEvent as ReactDragEvent } from "react";
import type { ClipSummary } from "../lib/types";
import { ClipActionButton } from "./ClipActionButton";
import { AnimatedImage } from "./AnimatedImage";
import { clipRowId } from "./ClipList";
import { ColorSwatch, displayHex } from "./ColorSwatch";
import { SecretPreview } from "./SecretPreview";
import { TagChip } from "./TagChip";
import { tagColorOf } from "../lib/tagColor";
import { IconCopy, IconFiles, IconFolder, IconGif, IconImage, IconNote, IconPin, IconShield, IconText, IconTrash } from "./icons";

interface ClipGridProps {
  items: ClipSummary[];
  selectedId: number | null;
  /** Ids picked with Ctrl/Shift for a bulk action. */
  selectedIds: number[];
  loading: boolean;
  emptyMessage: string;
  now: number;
  /** Card size multiplier; 1 is the designed size. */
  scale: number;
  /** Tag name (lower-cased) to its stored colour, when it has one. */
  tagColors: ReadonlyMap<string, string | null>;
  onSelect: (id: number, modifiers: { ctrl: boolean; shift: boolean }) => void;
  onActivate: (clip: ClipSummary) => void;
  onCopy: (clip: ClipSummary) => void;
  /** Copies the text recognised in an image clip. */
  onCopyText: (clip: ClipSummary) => void;
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
  slot,
  selected,
  multi,
  now,
  scale,
  tagColors,
  onSelect,
  onActivate,
  onCopy,
  onCopyText,
  onContextMenu,
  onTogglePin,
  onDelete,
  onRunAction,
  onPrefetch,
  onDragStart,
  showContent = true,
  showNotes = true,
}: {
  clip: ClipSummary;
  slot?: number;
  selected: boolean;
  multi: boolean;
  now: number;
  scale: number;
  tagColors: ReadonlyMap<string, string | null>;
  onSelect: (modifiers: { ctrl: boolean; shift: boolean }) => void;
  onActivate: () => void;
  onCopy: () => void;
  onCopyText: () => void;
  onContextMenu: (event: ReactMouseEvent) => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onRunAction: (action: ClipAction) => void;
  onPrefetch: () => void;
  onDragStart: (event: ReactDragEvent) => void;
  showContent: boolean;
  showNotes: boolean;
}) {
  const { mediaHeight } = geometry(scale);
  // A colour clip's caption repeats the copied code, so it takes the same
  // uppercase presentation as the swatch instead of the raw payload.
  const caption = clip.color ? displayHex(clip.preview) : clip.preview;
  // The card already shows the payload, masked, above the caption. Repeating it
  // here would put the secret in plain text underneath its own blur.
  const secret = Boolean(clip.secretLevel);
  const captionText = secret
    ? "Content hidden"
    : !showContent
      ? "Content hidden"
      : clip.kind === "image"
        ? clip.width && clip.height
          ? `${clip.width} × ${clip.height}`
          : "Image"
        : caption || "(empty)";
  const KindIcon =
    clip.contentKind === "path"
      ? IconFolder
      : clip.kind === "files"
        ? IconFiles
        : clip.kind === "image"
          ? IconImage
          : IconText;
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
        // Right-click keeps the selection intact for the bulk menu.
        if (event.button !== 0) return;
        onSelect({ ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey });
      }}
      onDoubleClick={onActivate}
      onContextMenu={onContextMenu}
      className={`group relative flex cursor-default flex-col overflow-hidden rounded-lg border transition-colors ${
        selected || multi
          ? "border-accent bg-accent/10"
          : "border-edge bg-panel-2 hover:border-edge-soft hover:bg-panel-3"
      }`}
    >
      <div
        className="relative shrink-0 overflow-hidden bg-canvas/70"
        style={{ height: mediaHeight }}
      >
        {clip.color ? (
          <ColorSwatch
            color={clip.color}
            label={clip.color}
            showRgb
            className="h-full w-full rounded-none border-0"
            textClassName="text-[11px]"
          />
        ) : clip.mediaLocked ? (
          // Encrypted while the vault is locked: there is no image to fetch, so the
          // card says why instead of showing a broken frame.
          <div className="flex h-full w-full flex-col items-center justify-center gap-1 bg-panel-3 text-warn">
            <IconShield className="size-4" />
            <span className="text-[9px] uppercase tracking-wide">Locked</span>
          </div>
        ) : clip.kind === "image" && clipImageUrl(clip) ? (
          <AnimatedImage
            still={clip.thumbFile ? mediaUrl(clip.thumbFile) : null}
            animated={clip.animated ? mediaUrl(`${clip.id}.gif`) : null}
            className="h-full w-full object-cover"
          />
        ) : (
          <p className="h-full w-full overflow-hidden whitespace-pre-wrap break-words p-2 text-[10px] leading-[1.45] text-ink-dim">
              <SecretPreview
                level={clip.secretLevel}
                corner
                text={showContent ? clip.preview : undefined}
              >
                {showContent
                  ? clip.preview || <span className="text-ink-faint">(empty)</span>
                  : <span className="text-ink-faint">Content hidden</span>}
              </SecretPreview>
            </p>
        )}

        {!clip.color && clip.kind !== "image" && (
          <KindIcon className="absolute bottom-1 right-1 size-3 text-ink-faint/50" />
        )}

        {clip.animated && (
          <span
            title="Animated GIF"
            aria-label="Animated GIF"
            className={`absolute top-1 rounded bg-canvas/80 p-0.5 text-accent-hi ${
              clip.secretLevel ? "left-6" : "left-1"
            }`}
          >
            <IconGif className="size-3" />
          </span>
        )}

        {clip.pinned && (
          <span
            className={`absolute top-1 rounded bg-canvas/80 p-0.5 text-warn ${
              clip.animated
                ? clip.secretLevel
                  ? "left-12"
                  : "left-7"
                : clip.secretLevel
                  ? "left-6"
                  : "left-1"
            }`}
          >
            <IconPin className="size-3" filled />
          </span>
        )}

        {showNotes && clip.hasNote && (
          <span
            title="Has a note"
            aria-label="Has a note"
            className="absolute bottom-1 left-1 rounded bg-canvas/80 p-0.5 text-accent"
          >
            <IconNote className="size-3" />
          </span>
        )}

        {slot !== undefined && (
          <span
            aria-hidden="true"
            className={`absolute right-1 rounded border border-edge bg-panel-3 px-1 text-[9px] font-semibold tabular-nums text-ink shadow-sm ${
              clip.pinned ? "top-1" : "bottom-1"
            }`}
          >
            {slot}
          </span>
        )}

        <div className="absolute right-1 top-1 flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          {action && (
            <ClipActionButton
              action={action}
              onRun={onRunAction}
              className="rounded bg-canvas/85 p-1 text-ink-dim hover:text-ink"
              iconClassName="size-3"
            />
          )}
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
        {/* No `title` for a secret: a tooltip is just the text again. */}
        <p className="truncate text-[11px] text-ink" title={secret ? undefined : captionText}>
          {captionText}
        </p>

        <div className="flex items-center gap-1 text-[9px] text-ink-faint">
          <span className={clipBadgeClass(clip)}>{clipLabel(clip)}</span>
          <span className="min-w-0 flex-1 truncate tabular-nums">
            {formatRelative(clip.updatedAt, now)}
          </span>
          {clip.copyCount > 1 && <span className="shrink-0 tabular-nums">×{clip.copyCount}</span>}
        </div>

        {clip.tags.length > 0 && (
          <div className="flex flex-wrap gap-1">
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
    </div>
  );
}

export function ClipGrid({
  items,
  selectedId,
  selectedIds,
  loading,
  emptyMessage,
  now,
  scale,
  tagColors,
  onSelect,
  onActivate,
  onCopy,
  onCopyText,
  onContextMenu,
  onTogglePin,
  onDelete,
  onRunAction,
  onPrefetch,
  onDragStart,
  showContent = true,
  showNotes = true,
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
        {items.map((clip, index) => (
          <Card
            key={clip.id}
            clip={clip}
            slot={index < 9 ? index + 1 : undefined}
            selected={clip.id === selectedId}
            multi={selectedIds.includes(clip.id)}
            now={now}
            scale={scale}
            tagColors={tagColors}
            onSelect={(modifiers) => onSelect(clip.id, modifiers)}
            onActivate={() => onActivate(clip)}
            onCopy={() => onCopy(clip)}
            onCopyText={() => onCopyText(clip)}
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
      </div>

      {loading && (
        <p className="px-3 py-4 text-center text-[11px] text-ink-faint">Loading…</p>
      )}
    </div>
  );
}
