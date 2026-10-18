import { useEffect, useState, type ReactNode } from "react";

import { formatBytes, formatCount, formatRelative, KIND_LABEL, leafName, shortenMiddle } from "../lib/format";
import { mediaUrl } from "../lib/ipc";
import type { ClipDetail, ClipSummary, Collection } from "../lib/types";
import { IconChevronLeft, IconChevronRight, IconCopy, IconPin, IconTrash } from "./icons";

interface ClipDetailPaneProps {
  clip: ClipSummary | null;
  detail: ClipDetail | null;
  loadingDetail: boolean;
  collections: Collection[];
  now: number;
  /** Whether the pane is expanded; collapsed leaves a narrow rail. */
  open: boolean;
  onToggle: () => void;
  onCopy: (clip: ClipSummary, autoPaste: boolean) => void;
  onTogglePin: (clip: ClipSummary) => void;
  onDelete: (clip: ClipSummary) => void;
  onTagsCommit: (clip: ClipSummary, tags: string[]) => void;
  onCollectionsChange: (clip: ClipSummary, ids: number[]) => void;
}

function TagEditor({
  tags,
  onCommit,
}: {
  tags: string[];
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
          <button
            key={tag}
            type="button"
            title="Remove tag"
            onClick={() => onCommit(tags.filter((existing) => existing !== tag))}
            className="group flex items-center gap-1 rounded bg-accent/10 px-1.5 py-0.5 text-[10px] text-accent-hi hover:bg-danger/20 hover:text-danger"
          >
            #{tag}
            <span aria-hidden="true" className="opacity-60">
              ×
            </span>
          </button>
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

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-t border-edge-soft px-3 py-2.5 first:border-t-0">
      <h3 className="pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
        {title}
      </h3>
      {children}
    </section>
  );
}

export function ClipDetailPane({
  clip,
  detail,
  loadingDetail,
  collections,
  now,
  open,
  onToggle,
  onCopy,
  onTogglePin,
  onDelete,
  onTagsCommit,
  onCollectionsChange,
}: ClipDetailPaneProps) {
  if (!open) {
    return (
      <aside className="flex w-7 shrink-0 flex-col items-center border-l border-edge bg-panel py-1.5">
        <button
          type="button"
          onClick={onToggle}
          title="Show the preview pane"
          aria-label="Show the preview pane"
          aria-expanded={false}
          className="rounded p-1 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconChevronLeft className="size-3.5" />
        </button>
      </aside>
    );
  }

  if (!clip) {
    return (
      <aside className="flex w-[286px] shrink-0 flex-col border-l border-edge bg-panel">
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

  return (
    <aside className="flex w-[286px] shrink-0 flex-col border-l border-edge bg-panel">
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
          Copy
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

      <div className="min-h-0 flex-1 overflow-y-auto">
        <Section title="Content">
          {loadingDetail && <p className="text-[11px] text-ink-faint">Decrypting…</p>}

          {!loadingDetail && clip.kind === "text" && (
            <pre
              data-selectable
              className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-panel-2 p-2 font-mono text-[11px] leading-relaxed text-ink"
            >
              {detail?.text ?? clip.preview}
            </pre>
          )}

          {!loadingDetail && clip.kind === "image" && clip.imageFile && (
            <img
              src={mediaUrl(clip.imageFile)}
              alt="Clipboard image"
              className="max-h-72 w-full rounded border border-edge object-contain"
            />
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

        <Section title="Details">
          <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-[10px]">
            <dt className="text-ink-faint">Type</dt>
            <dd className="text-ink-dim">{KIND_LABEL[clip.kind]}</dd>

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

        <Section title="Collections">
          {collections.length === 0 ? (
            <p className="text-[11px] text-ink-faint">
              No collections yet — create one in the sidebar.
            </p>
          ) : (
            <ul className="space-y-0.5">
              {collections.map((collection) => (
                <li key={collection.id}>
                  <label className="flex cursor-default items-center gap-2 rounded px-1 py-1 text-[11px] text-ink-dim hover:bg-panel-2">
                    <input
                      type="checkbox"
                      checked={clip.collectionIds.includes(collection.id)}
                      onChange={() => toggleCollection(collection.id)}
                      className="size-3 accent-[var(--color-accent)]"
                    />
                    <span className="min-w-0 flex-1 truncate">{collection.name}</span>
                  </label>
                </li>
              ))}
            </ul>
          )}
        </Section>

        <Section title="Tags">
          <TagEditor tags={clip.tags} onCommit={(tags) => onTagsCommit(clip, tags)} />
        </Section>
      </div>
    </aside>
  );
}
