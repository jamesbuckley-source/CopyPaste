import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { formatBytes, formatCount, formatRelative, clipFullImageUrl, clipLabel, leafName, shortenMiddle } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { ClipDetail, ClipSummary, Collection } from "../lib/types";
import { useSectionOrder, type SectionHandle } from "../lib/sectionOrder";
import { AnimatedImage } from "./AnimatedImage";
import { ColorSwatch, displayHex, rgbLabel } from "./ColorSwatch";
import { secretLabel } from "./SecretPreview";
import { TagChip } from "./TagChip";
import { tagColorOf } from "../lib/tagColor";
import {
  IconChevronLeft,
  IconChevronRight,
  IconCopy,
  IconExternal,
  IconFolder,
  IconGif,
  IconGrip,
  IconMail,
  IconNote,
  IconPin,
  IconSave,
  IconSearch,
  IconShield,
  IconTrash,
} from "./icons";

interface ClipDetailPaneProps {
  clip: ClipSummary | null;
  detail: ClipDetail | null;
  loadingDetail: boolean;
  collections: Collection[];
  /** How many collections the linked lists show before "Show all". */
  listLimit: number;
  now: number;
  /** Tag name (lower-cased) to its stored colour, when it has one. */
  tagColors: ReadonlyMap<string, string | null>;
  /** Whether the pane is expanded; collapsed leaves a narrow rail. */
  open: boolean;
  /** Section ids in the remembered drag order. */
  sectionOrder: string[];
  onSectionOrderChange: (order: string[]) => void;
  /** Rich flavours captured with the clip, when it has any. */
  onToggle: () => void;
  onCopy: (clip: ClipSummary, autoPaste: boolean) => void;
  /** Copies the text recognised in an image clip. */
  onCopyText: (clip: ClipSummary) => void;
  /** Copies a colour from an image's generated scheme. */
  onCopyColor: (hex: string) => void;
  /** Files the generated scheme into a new palette archive. */
  onSavePalette: (colors: string[]) => void;
  /** The selected image's colour scheme, or null while it is being read. */
  schemeColors: string[] | null;
  schemeBusy: boolean;
  schemeError: string | null;
  onTogglePin: (clip: ClipSummary) => void;
  onDelete: (clip: ClipSummary) => void;
  onTagsCommit: (clip: ClipSummary, tags: string[]) => void;
  /** Marks a text clip as a secret, or clears the mark. */
  onToggleSecret: (clip: ClipSummary) => void;
  onCollectionsChange: (clip: ClipSummary, ids: number[]) => void;
  /** Writes the payload into a folder the user picks. */
  onSaveAs: (clip: ClipSummary) => void;
  /** Writes a text clip into the Markdown vault as a note. */
  onExportMarkdown: (clip: ClipSummary) => void;
  /** Opens a link clip in the default browser. */
  onOpenUrl: (url: string) => void;
  /** Opens the default mail client for an email clip. */
  onOpenEmail: (email: string) => void;
  /** Opens a path clip with its default application. */
  onOpenPath: (path: string) => void;
  /** Reveals a path clip in Explorer. */
  onRevealPath: (path: string) => void;
  /** Archive collections, offered as the clip's single archive slot. */
  archives: Collection[];
  /** Current pane width in CSS pixels. */
  width: number;
  /** Reports the width while the edge is being dragged. */
  onWidthChange: (width: number) => void;
  /** Persists the width once the drag ends. */
  onWidthCommit: (width: number) => void;
  /** Files the clip into an archive, or `null` to take it out. */
  onArchiveChange: (clip: ClipSummary, archiveId: number | null) => void;
  /** Adds the clip's image to the GIF shelf, or removes it again. */
  onToggleGif: (clip: ClipSummary) => void;
  /** True when this clip already sits on the GIF shelf. */
  gifFavorite: boolean;
  /** Saves or clears the clip's note. */
  onNoteCommit: (clip: ClipSummary, note: string | null) => void;
  /** Replaces a text clip's body after an inline edit. */
  onTextCommit: (clip: ClipSummary, text: string) => void;
}

function TagEditor({
  tags,
  tagColors,
  onCommit,
}: {
  tags: string[];
  tagColors: ReadonlyMap<string, string | null>;
  onCommit: (tags: string[]) => void;
}) {
  const [draft, setDraft] = useState("");

  // Reset the field when the selection moves to a different clip.
  useEffect(() => setDraft(""), [tags]);

  const commit = () => {
    const additions = draft
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    if (additions.length === 0) return;
    const next = [...tags];
    for (const addition of additions) {
      if (!next.some((tag) => tag.toLowerCase() === addition.toLowerCase())) {
        next.push(addition);
      }
    }
    onCommit(next);
    setDraft("");
  };

  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap gap-1">
        {tags.map((tag) => (
          <TagChip
            key={tag}
            name={tag}
            color={tagColorOf(tagColors, tag)}
            title="Remove tag"
            onClick={() => onCommit(tags.filter((existing) => existing !== tag))}
            className="group flex items-center gap-1 px-1.5 py-0.5 text-[10px] hover:bg-danger/20 hover:text-danger"
          >
            <span aria-hidden="true" className="opacity-60">
              ×
            </span>
          </TagChip>
        ))}
      </div>
      <input
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            commit();
            event.preventDefault();
          }
          // Keep global shortcuts (Delete, arrows) from acting while typing.
          event.stopPropagation();
        }}
        onBlur={commit}
        placeholder={tags.length ? "Add tag…" : "Add tags, comma separated"}
        aria-label="Add tags"
        spellCheck={false}
        className="w-full rounded border border-edge bg-panel-2 px-2 py-1 text-[11px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
      />
    </div>
  );
}

/** Picks collections for a clip: attached ones become removable pills. */
function CollectionPicker({
  collections,
  selectedIds,
  listLimit,
  onToggle,
}: {
  collections: Collection[];
  selectedIds: number[];
  listLimit: number;
  onToggle: (collectionId: number) => void;
}) {
  const [filter, setFilter] = useState("");
  const [showAll, setShowAll] = useState(false);
  const filterRef = useRef<HTMLInputElement>(null);

  const toggle = (collectionId: number) => {
    onToggle(collectionId);
    // The row turns into a pill, so hand focus back to the field rather than
    // dropping it on <body>.
    filterRef.current?.focus({ preventScroll: true });
  };

  const active = collections.filter((collection) => !collection.archived);
  const attached = selectedIds
    .map((id) => active.find((collection) => collection.id === id))
    .filter((collection): collection is Collection => collection !== undefined);
  const query = filter.trim().toLowerCase();
  const matching = active
    .filter((collection) => !selectedIds.includes(collection.id))
    .filter((collection) => !query || collection.name.toLowerCase().includes(query));
  const limit = Math.max(1, listLimit);
  const visible = showAll ? matching : matching.slice(0, limit);

  return (
    <div className="space-y-1.5">
      {attached.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {attached.map((collection) => (
            <button
              key={collection.id}
              type="button"
              title="Remove from this collection"
              onClick={() => toggle(collection.id)}
              className="flex items-center gap-1 rounded bg-accent/10 px-1.5 py-0.5 text-[10px] text-accent-hi hover:bg-danger/20 hover:text-danger"
            >
              <span className="max-w-32 truncate">{collection.name}</span>
              <span aria-hidden="true" className="opacity-60">
                ×
              </span>
            </button>
          ))}
        </div>
      )}
      <div className="relative">
        <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
        <input
          ref={filterRef}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          onKeyDown={(event) => {
            // Keep global shortcuts (Delete, arrows) from acting while typing.
            event.stopPropagation();
            if (event.key === "Escape") {
              event.preventDefault();
              setFilter("");
            }
          }}
          placeholder="Filter collections"
          aria-label="Filter collections"
          spellCheck={false}
          autoComplete="off"
          className="w-full rounded border border-edge bg-panel-2 py-1 pl-6 pr-1.5 text-[10px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
        />
      </div>
      {visible.length > 0 && (
        <ul className="space-y-0.5">
          {visible.map((collection) => (
            <li key={collection.id}>
              <label className="flex cursor-default items-center gap-2 rounded px-1 py-1 text-[11px] text-ink-dim hover:bg-panel-2">
                <input
                  type="checkbox"
                  checked={false}
                  onChange={() => toggle(collection.id)}
                  className="size-3 accent-[var(--color-accent)]"
                />
                <span className="min-w-0 flex-1 truncate">{collection.name}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
      {matching.length === 0 && (
        <p className="px-1 text-[10px] text-ink-faint">
          {query
            ? "No collection matches."
            : attached.length > 0
              ? "The clip is in every collection."
              : "No collections left to add."}
        </p>
      )}
      {matching.length > limit && (
        <button
          type="button"
          onClick={() => setShowAll((shown) => !shown)}
          className="w-full rounded px-1 py-0.5 text-left text-[10px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
        >
          {showAll ? "Show fewer" : `Show all ${matching.length}`}
        </button>
      )}
    </div>
  );
}

function NoteEditor({
  note,
  onCommit,
}: {
  note: string | null;
  onCommit: (note: string | null) => void;
}) {
  const [draft, setDraft] = useState(note ?? "");
  // The detail payload is re-read after every save, so follow it when it changes.
  useEffect(() => setDraft(note ?? ""), [note]);

  const commit = () => {
    const next = draft.trim();
    if (next === (note ?? "").trim()) return;
    onCommit(next.length > 0 ? next : null);
  };

  return (
    <div className="space-y-1">
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
            event.preventDefault();
            commit();
            event.currentTarget.blur();
          }
          // Keep global shortcuts (Delete, arrows) from acting while typing.
          event.stopPropagation();
        }}
        rows={3}
        maxLength={4_000}
        placeholder="Add a note to remember why this clip matters…"
        aria-label="Clip note"
        className="w-full resize-y rounded border border-edge bg-panel-2 px-2 py-1.5 text-[11px] leading-relaxed text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
      />
      <p className="text-[10px] leading-relaxed text-ink-faint">
        Saved when you leave the field, or with Ctrl+Enter. Notes are searchable.
      </p>
    </div>
  );
}

/**
 * Text body that becomes an editor on double-click. `value` is the raw payload
 * (what gets saved) while `display` may be a formatted view of the same text.
 */
function EditableText({
  display,
  value,
  onCommit,
  className,
}: {
  display: string;
  value: string | null;
  onCommit: (text: string) => void;
  className: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value ?? "");

  const start = () => {
    if (value === null) return;
    setDraft(value);
    setEditing(true);
  };
  const cancel = () => {
    setDraft(value ?? "");
    setEditing(false);
  };
  const save = () => {
    if (draft.trim().length === 0) return;
    setEditing(false);
    if (draft !== value) onCommit(draft);
  };

  if (editing) {
    return (
      <div className="space-y-1">
        <textarea
          autoFocus
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              cancel();
            } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
              event.preventDefault();
              save();
            }
            event.stopPropagation();
          }}
          spellCheck={false}
          aria-label="Edit clip text"
          className="min-h-28 w-full resize-y rounded border border-accent-dim bg-panel-2 p-2 font-mono text-[11px] leading-relaxed text-ink focus:outline-none"
        />
        <div className="flex items-center justify-between gap-2">
          <span className="text-[10px] text-ink-faint">Ctrl+Enter saves · Esc cancels</span>
          <div className="flex gap-1">
            <button
              type="button"
              onClick={cancel}
              className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim hover:text-ink"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={save}
              disabled={draft.trim().length === 0}
              className="rounded border border-accent/60 bg-accent/20 px-2 py-0.5 text-[10px] text-accent-hi hover:bg-accent/30 disabled:opacity-40"
            >
              Save
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <pre
      data-selectable
      onDoubleClick={value === null ? undefined : start}
      title={value === null ? undefined : "Double-click to edit"}
      className={`${className} ${value === null ? "" : "cursor-text"}`}
    >
      {display}
    </pre>
  );
}

/** Section ids in the preview pane, used for the remembered drag order. */
const SECTIONS = ["content", "details", "note", "collections", "archive", "tags"] as const;

function Section({
  id,
  title,
  handle,
  children,
}: {
  id: string;
  title: string;
  handle?: SectionHandle;
  children: ReactNode;
}) {
  const line = handle?.lineBefore
    ? "border-t-2 border-accent"
    : handle?.lineAfter
      ? "border-b-2 border-accent"
      : "border-t border-edge-soft first:border-t-0";

  return (
    <section
      ref={handle?.register}
      data-section={id}
      style={handle === undefined ? undefined : { order: handle.order }}
      className={`group px-3 py-2.5 ${line} ${handle?.dragging ? "opacity-60" : ""}`}
    >
      <h3 className="flex items-center pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
        {title}
        {handle && (
          <span
            role="button"
            tabIndex={-1}
            aria-label={`Move ${title.toLowerCase()}`}
            title="Drag to re-order"
            {...handle.grip}
            className="ml-auto flex w-4 shrink-0 cursor-grab touch-none items-center justify-center text-ink-faint/60 transition-colors hover:text-ink-dim active:cursor-grabbing"
          >
            <IconGrip className="size-3" />
          </span>
        )}
      </h3>
      {children}
    </section>
  );
}

/** Compact button for the content-aware actions offered under a preview. */
function ActionButton({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon?: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1 rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
    >
      {icon}
      {label}
    </button>
  );
}

/** Displays the dominant colours of an image clip, as read by the app shell. */
function ColorScheme({
  colors,
  busy,
  error,
  onCopy,
  onSavePalette,
}: {
  /** The scheme of the clip on screen, or null while it is still being read. */
  colors: string[] | null;
  busy: boolean;
  error: string | null;
  onCopy: (hex: string) => void;
  onSavePalette: (colors: string[]) => void;
}) {
  const hasColors = colors !== null && colors.length > 0;

  return (
    <div className="mt-2 rounded border border-edge bg-panel-2 p-2">
      <div className="mb-1.5 flex items-center justify-between">
        <h4 className="text-[9px] font-semibold uppercase tracking-wider text-ink-faint">
          Color scheme
        </h4>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => colors && onSavePalette(colors)}
            disabled={busy || !hasColors}
            title={
              hasColors
                ? "Save these colors into a new Color Palette archive"
                : "Waiting for the colors to be read"
            }
            className="flex items-center gap-1 rounded px-1 py-0.5 text-[10px] text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-50"
          >
            <IconSave className="size-3" />
            {busy ? "Reading…" : "Make Color Palette"}
          </button>
        </div>
      </div>
      {hasColors ? (
        <div className="grid grid-cols-5 gap-1">
          {colors.map((hex) => (
            <button
              key={hex}
              type="button"
              onClick={() => onCopy(hex)}
              title={`Copy ${displayHex(hex)} · ${rgbLabel(hex)}`}
              className="min-w-0 rounded transition-transform hover:-translate-y-px"
            >
              <ColorSwatch
                color={hex}
                label={hex}
                className="h-12 w-full"
                textClassName="text-[8px]"
              />
              <span className="mt-0.5 block truncate font-mono text-[8px] text-ink-faint">
                {rgbLabel(hex)}
              </span>
            </button>
          ))}
        </div>
      ) : (
        <p className="text-[10px] leading-relaxed text-ink-faint">
          {error ?? (busy ? "Reading the dominant colors…" : "This image has no colors to read.")}
        </p>
      )}
    </div>
  );
}

export function ClipDetailPane({
  clip,
  detail,
  loadingDetail,
  collections,
  listLimit,
  now,
  tagColors,
  open,
  sectionOrder,
  onSectionOrderChange,
  onToggle,
  onCopy,
  onCopyText,
  onCopyColor,
  onSavePalette,
  schemeColors,
  schemeBusy,
  schemeError,
  onTogglePin,
  onDelete,
  onTagsCommit,
  onToggleSecret,
  onCollectionsChange,
  onSaveAs,
  onExportMarkdown,
  onOpenUrl,
  onOpenEmail,
  onOpenPath,  onRevealPath,  archives,
  width: storedWidth,
  onWidthChange,
  onWidthCommit,
  onArchiveChange,
  onToggleGif,
  gifFavorite,
  onNoteCommit,
  onTextCommit,
}: ClipDetailPaneProps) {
  // Hooks must run before the early returns below, so state lives at the top.
  const [prettyJson, setPrettyJson] = useState(false);
  const [liveWidth, setLiveWidth] = useState<number | null>(null);
  // A credential stays blurred until the user asks to read it.
  const [secretShown, setSecretShown] = useState(false);
  // Marked secret, and the vault is locked, so there is no text to blur: the pane
  // says that instead of showing an empty box.
  const secretLocked =
    Boolean(clip?.secretLevel) && !loadingDetail && (detail?.text ?? clip?.preview ?? "") === "";
  const secretBlurred = Boolean(clip?.secretLevel) && !secretShown && !secretLocked;
  useEffect(() => setSecretShown(false), [clip?.id]);
  // The drag handlers are plain DOM listeners, so the latest width lives in a ref
  // rather than being read back out of state.
  const liveWidthRef = useRef<number | null>(null);
  useEffect(() => setPrettyJson(false), [clip?.id]);
  // Once the stored width catches up with the drag, hand control back to settings.
  useEffect(() => {
    if (liveWidth !== null && storedWidth === liveWidth) setLiveWidth(null);
  }, [storedWidth, liveWidth]);

  const width = liveWidth ?? storedWidth;
  const widthStyle = { width, maxWidth: "80%" } as const;
  /** Below this the action row keeps its icons and gives up the Copy label. */
  const compact = width < 340;

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;
    const limit = Math.round(window.innerWidth * 0.8);
    const move = (moveEvent: PointerEvent) => {
      const next = Math.min(limit, Math.max(200, startWidth - (moveEvent.clientX - startX)));
      liveWidthRef.current = next;
      setLiveWidth(next);
      onWidthChange(next);
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      const current = liveWidthRef.current;
      if (current !== null) onWidthCommit(current);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  };

  const resizeHandle = (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the preview pane"
      title="Drag to resize the preview pane"
      onPointerDown={startResize}
      className="absolute inset-y-0 left-0 z-10 w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
    />
  );

  const sections = useSectionOrder(sectionOrder, SECTIONS, onSectionOrderChange);

  if (!open) {
    // The whole rail is the expand target, not just the glyph: it is a narrow
    // strip, so asking the user to hit a 14px chevron is needless precision.
    return (
      <aside className="flex w-7 shrink-0 flex-col border-l border-edge bg-panel">
        <button
          type="button"
          onClick={onToggle}
          title="Show the preview pane"
          aria-label="Show the preview pane"
          aria-expanded={false}
          className="flex h-full w-full flex-1 items-start justify-center pt-1.5 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconChevronLeft className="size-3.5" />
        </button>
      </aside>
    );
  }

  if (!clip) {
    return (
      <aside
        style={widthStyle}
        className="relative flex shrink-0 flex-col border-l border-edge bg-panel"
      >
        {resizeHandle}
        <div className="flex shrink-0 items-center gap-1.5 border-b border-edge-soft px-1.5 py-1.5">
          <button
            type="button"
            onClick={onToggle}
            title="Collapse the preview pane"
            aria-label="Collapse the preview pane"
            aria-expanded
            className="flex items-center justify-center rounded p-1 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
          >
            <IconChevronRight className="size-3.5" />
          </button>
          <span className="text-[10px] font-medium uppercase tracking-wider text-ink-faint">
            Preview
          </span>
        </div>
        <div className="flex flex-1 items-center justify-center px-5 text-center">
          <p className="text-[11px] text-ink-faint">
            Select a clip to preview it and to file it under a collection.
          </p>
        </div>
      </aside>
    );
  }

  const toggleCollection = (collectionId: number) => {
    const selected = clip.collectionIds.includes(collectionId);
    const next = selected
      ? clip.collectionIds.filter((id) => id !== collectionId)
      : [...clip.collectionIds, collectionId];
    onCollectionsChange(clip, next);
  };

  const body = detail?.text ?? clip.preview;
  // Pretty-printing is a view concern: the stored payload is never rewritten.
  const prettyBody = (() => {
    if (!prettyJson) return body;
    try {
      return JSON.stringify(JSON.parse(body), null, 2);
    } catch {
      return body;
    }
  })();
  const contentKind = clip.contentKind;
  const archiveIds = new Set(archives.map((archive) => archive.id));
  const currentArchive = clip.collectionIds.find((id) => archiveIds.has(id)) ?? null;

  return (
    <aside
      style={widthStyle}
      className="relative flex shrink-0 flex-col border-l border-edge bg-panel"
    >
      {resizeHandle}
      <div className="flex shrink-0 items-center gap-1.5 border-b border-edge-soft px-1.5 py-1.5">
        <button
          type="button"
          onClick={onToggle}
          title="Collapse the preview pane"
          aria-label="Collapse the preview pane"
          aria-expanded
          className="flex items-center justify-center rounded p-1 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconChevronRight className="size-3.5" />
        </button>
        <button
          type="button"
          onClick={() => onCopy(clip, false)}
          title="Copy to clipboard"
          className="flex flex-1 items-center justify-center gap-1.5 rounded bg-accent/15 px-2 py-1.5 text-[11px] font-medium text-accent-hi transition-colors hover:bg-accent/25"
        >
          <IconCopy className="size-3.5" />
          {/* A narrow pane keeps every action and drops the word instead. */}
          {!compact && "Copy"}
        </button>
        <button
          type="button"
          onClick={() => onTogglePin(clip)}
          title={clip.pinned ? "Unpin" : "Pin to the top"}
          aria-label={clip.pinned ? "Unpin clip" : "Pin clip"}
          className={`flex items-center justify-center rounded px-2 py-1.5 transition-colors ${
            clip.pinned ? "bg-warn/20 text-warn" : "text-ink-dim hover:bg-panel-3 hover:text-ink"
          }`}
        >
          <IconPin className="size-3.5" filled={clip.pinned} />
        </button>
        {clip.kind === "image" && (
          <button
            type="button"
            onClick={() => onToggleGif(clip)}
            title={gifFavorite ? "Remove from My GIFs" : "Add to My GIFs"}
            aria-label={gifFavorite ? "Remove from My GIFs" : "Add to My GIFs"}
            aria-pressed={gifFavorite}
            className={`flex items-center justify-center rounded px-2 py-1.5 transition-colors ${
              gifFavorite ? "bg-warn/20 text-warn" : "text-ink-dim hover:bg-panel-3 hover:text-ink"
            }`}
          >
            <IconGif className="size-3.5" />
          </button>
        )}
        {clip.kind === "text" && (
          <button
            type="button"
            onClick={() => onToggleSecret(clip)}
            title={clip.secretLevel ? "Remove the secret mark" : "Mark this clip as a secret"}
            aria-label={clip.secretLevel ? "Remove the secret mark" : "Mark clip as a secret"}
            aria-pressed={Boolean(clip.secretLevel)}
            className={`flex items-center justify-center rounded px-2 py-1.5 transition-colors ${
              clip.secretLevel
                ? "bg-warn/20 text-warn"
                : "text-ink-dim hover:bg-panel-3 hover:text-ink"
            }`}
          >
            <IconShield className="size-3.5" />
          </button>
        )}
        <button
          type="button"
          onClick={() => onSaveAs(clip)}
          title="Save this clip to a folder"
          aria-label="Save clip to a folder"
          className="flex items-center justify-center rounded px-2 py-1.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconSave className="size-3.5" />
        </button>
        {clip.kind === "text" && (
          <button
            type="button"
            onClick={() => onExportMarkdown(clip)}
            title="Export as a Markdown note"
            aria-label="Export as a Markdown note"
            className="flex items-center justify-center rounded px-2 py-1.5 text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
          >
            <IconNote className="size-3.5" />
          </button>
        )}
        <button
          type="button"
          onClick={() => onDelete(clip)}
          title="Delete (Del)"
          aria-label="Delete clip"
          className="flex items-center justify-center rounded px-2 py-1.5 text-ink-dim transition-colors hover:bg-danger/20 hover:text-danger"
        >
          <IconTrash className="size-3.5" />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        <Section
          id="content"
          title="Content"
          handle={sections.handle("content")}
        >
          {loadingDetail && <p className="text-[11px] text-ink-faint">Decrypting…</p>}

          {!loadingDetail && clip.secretLevel && (
            <p className="mb-1.5 flex items-center gap-1.5 rounded border border-warn/40 bg-warn/10 px-2 py-1 text-[10px] leading-relaxed text-warn">
              <IconShield className="size-3 shrink-0" />
              <span className="min-w-0 flex-1">
                {secretLabel(clip.secretLevel)}. Stored as-is, but hidden until you ask.
              </span>
              <button
                type="button"
                onClick={() => setSecretShown(!secretShown)}
                className="shrink-0 rounded border border-warn/40 px-1.5 py-px transition-colors hover:bg-warn/20"
              >
                {secretShown ? "Hide" : "Reveal"}
              </button>
            </p>
          )}

          {!loadingDetail && clip.kind === "text" && clip.color && (
            <div className="space-y-2">
              <ColorSwatch
                color={clip.color}
                label={clip.color}
                showRgb
                className="h-24 w-full"
                textClassName="text-[12px]"
              />
              {secretLocked && (
                <p className="flex items-center gap-1.5 rounded border border-warn/30 bg-warn/10 px-2 py-1.5 text-[10px] leading-relaxed text-warn">
                  <IconShield className="size-3 shrink-0" />
                  Locked. Unlock the vault from the tray menu or Settings to read this clip.
                </p>
              )}
              <EditableText
                key={clip.id}
                display={secretLocked ? "" : (detail?.text ?? clip.preview)}
                value={loadingDetail ? null : (detail?.text ?? null)}
                onCommit={(text) => onTextCommit(clip, text)}
                className={`max-h-32 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-2 font-mono text-[11px] leading-relaxed text-ink ${
                  secretBlurred ? "blur-[3px] select-none" : ""
                }`}
              />
            </div>
          )}

          {!loadingDetail && clip.kind === "text" && !clip.color && (
            <div className="space-y-1.5">
              {(contentKind === "url" ||
                contentKind === "email" ||
                contentKind === "path" ||
                contentKind === "json") && (
                <div className="flex flex-wrap gap-1">
                  {contentKind === "url" && (
                    <ActionButton
                      label="Open link"
                      icon={<IconExternal className="size-3" />}
                      onClick={() => onOpenUrl(body.trim())}
                    />
                  )}
                  {contentKind === "email" && (
                    <ActionButton
                      label="Send email"
                      icon={<IconMail className="size-3" />}
                      onClick={() => onOpenEmail(body.trim())}
                    />
                  )}
                  {contentKind === "path" && (
                    <>
                      <ActionButton
                        label="Open"
                        icon={<IconExternal className="size-3" />}
                        onClick={() => onOpenPath(body.trim())}
                      />
                      <ActionButton
                        label="Open Folder"
                        icon={<IconFolder className="size-3" />}
                        onClick={() => onRevealPath(body.trim())}
                      />
                    </>
                  )}
                  {contentKind === "json" && (
                    <ActionButton
                      label={prettyJson ? "Show raw" : "Format JSON"}
                      onClick={() => setPrettyJson((current) => !current)}
                    />
                  )}
                </div>
              )}
              <EditableText
                key={clip.id}
                display={prettyBody}
                value={loadingDetail || prettyJson || !detail ? null : body}
                onCommit={(text) => onTextCommit(clip, text)}
                className={`max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-2 font-mono text-[11px] leading-relaxed text-ink ${
                  secretBlurred ? "blur-[3px] select-none" : ""
                }`}
              />
              {detail?.hasRich && (
                <p className="text-[10px] leading-relaxed text-ink-faint">
                  Copied with its formatting; pasting into a rich editor keeps it.
                </p>
              )}
            </div>
          )}

          {clip.kind === "image" && clipFullImageUrl(clip) && (
            <div className="space-y-2">
              <AnimatedImage
                still={clip.imageFile ? mediaUrl(clip.imageFile) : null}
                animated={clip.animated ? mediaUrl(`${clip.id}.gif`) : null}
                alt="Clipboard image"
                loading="eager"
                className="max-h-72 w-full rounded border border-edge object-contain"
              />
              {/* Kept mounted while the detail reloads, so copying a swatch does
                  not throw away the scheme the user just generated. */}
              <ColorScheme
                colors={schemeColors}
                busy={schemeBusy}
                error={schemeError}
                onCopy={onCopyColor}
                onSavePalette={onSavePalette}
              />
            </div>
          )}

          {!loadingDetail && clip.kind === "image" && detail?.ocrText && (
            <div className="mt-2">
              <div className="mb-1 flex items-center justify-between">
                <h4 className="text-[9px] font-semibold uppercase tracking-wider text-ink-faint">
                  Text in image
                </h4>
                <button
                  type="button"
                  onClick={() => onCopyText(clip)}
                  title="Copy the text in this image"
                  className="flex items-center gap-1 rounded px-1 py-0.5 text-[10px] text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
                >
                  <IconCopy className="size-3" />
                  Copy text
                </button>
              </div>
              <pre
                data-selectable
                className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-2 font-mono text-[10px] leading-relaxed text-ink-dim"
              >
                {detail.ocrText}
              </pre>
            </div>
          )}

          {!loadingDetail && clip.kind === "files" && (
            <ul className="space-y-1">
              {(detail?.filePaths ?? []).map((path) => (
                <li
                  key={path}
                  data-selectable
                  title={path}
                  className="truncate rounded bg-panel-2 px-2 py-1 font-mono text-[10px] text-ink-dim"
                >
                  {shortenMiddle(leafName(path), 34)}
                </li>
              ))}
              {detail && detail.filePaths.length === 0 && (
                <li className="text-[11px] text-ink-faint">No stored paths.</li>
              )}
            </ul>
          )}
        </Section>

        <Section
          id="details"
          title="Details"
          handle={sections.handle("details")}
        >
          <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-[10px]">
            <dt className="text-ink-faint">Type</dt>
            <dd className="text-ink-dim">{clipLabel(clip)}</dd>

            {clip.color && (
              <>
                <dt className="text-ink-faint">Hex</dt>
                <dd className="font-mono text-ink-dim">{displayHex(clip.color)}</dd>
                <dt className="text-ink-faint">RGB</dt>
                <dd className="font-mono text-ink-dim">{rgbLabel(clip.color)}</dd>
              </>
            )}

            <dt className="text-ink-faint">Size</dt>
            <dd className="text-ink-dim tabular-nums">{formatBytes(clip.byteSize)}</dd>

            {clip.kind === "text" && (
              <>
                <dt className="text-ink-faint">Length</dt>
                <dd className="text-ink-dim tabular-nums">
                  {formatCount(clip.charCount, "character")}
                </dd>
              </>
            )}

            {clip.kind === "image" && clip.width && clip.height && (
              <>
                <dt className="text-ink-faint">Pixels</dt>
                <dd className="text-ink-dim tabular-nums">
                  {clip.width} × {clip.height}
                </dd>
              </>
            )}

            {clip.kind === "image" && clip.faceCount > 0 && (
              <>
                <dt className="text-ink-faint">People</dt>
                <dd className="text-ink-dim tabular-nums">
                  {formatCount(clip.faceCount, "face")} found
                </dd>
              </>
            )}

            {clip.sourceApp && (
              <>
                <dt className="text-ink-faint">From</dt>
                <dd className="truncate text-ink-dim" title={clip.sourceApp}>
                  {clip.sourceApp}
                </dd>
              </>
            )}

            <dt className="text-ink-faint">Captured</dt>
            <dd className="text-ink-dim">{formatRelative(clip.createdAt, now)}</dd>

            <dt className="text-ink-faint">Reused</dt>
            <dd className="text-ink-dim tabular-nums">
              {formatCount(clip.copyCount, "time")}
            </dd>
          </dl>
        </Section>

        <Section
          id="note"
          title="Note"
          handle={sections.handle("note")}
        >
          <NoteEditor note={detail?.note ?? null} onCommit={(note) => onNoteCommit(clip, note)} />
        </Section>

        <Section
          id="collections"
          title="Add to Collections"
          handle={sections.handle("collections")}
        >
          {collections.some((collection) => !collection.archived) ? (
            <CollectionPicker
              key={clip.id}
              collections={collections}
              selectedIds={clip.collectionIds}
              listLimit={listLimit}
              onToggle={toggleCollection}
            />
          ) : (
            <p className="text-[11px] text-ink-faint">
              No collections yet — create one in the sidebar.
            </p>
          )}
          <p className="mt-1 text-[10px] leading-relaxed text-ink-faint">
            A clip can sit in as many collections as you like.
          </p>
        </Section>

        <Section
          id="archive"
          title="Archive"
          handle={sections.handle("archive")}
        >
          {archives.length === 0 ? (
            <p className="text-[11px] text-ink-faint">
              No archives yet — archive a collection from the sidebar to keep clips out of the
              history.
            </p>
          ) : (
            <label className="flex items-center gap-2 text-[11px] text-ink-dim">
              <span className="shrink-0">Archive</span>
              <select
                value={currentArchive ?? ""}
                onChange={(event) =>
                  onArchiveChange(
                    clip,
                    event.target.value === "" ? null : Number(event.target.value),
                  )
                }
                aria-label="Archive for this clip"
                className="min-w-0 flex-1 rounded border border-edge bg-panel-2 px-1.5 py-1 text-[11px] text-ink-dim focus:border-accent-dim focus:outline-none"
              >
                <option value="">Not archived</option>
                {archives.map((archive) => (
                  <option key={archive.id} value={archive.id}>
                    {archive.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          <p className="mt-1 text-[10px] leading-relaxed text-ink-faint">
            Archiving is a single location, so a clip has one archive at most.
          </p>
        </Section>

        <Section
          id="tags"
          title="Tags"
          handle={sections.handle("tags")}
        >
          <TagEditor
            tags={clip.tags}
            tagColors={tagColors}
            onCommit={(tags) => onTagsCommit(clip, tags)}
          />
        </Section>
      </div>
    </aside>
  );
}
