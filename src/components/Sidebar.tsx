import { useEffect, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";

import type { Collection, SidebarView, Stats, Tag } from "../lib/types";
import {
  IconAll,
  IconArchive,
  IconFiles,
  IconFolder,
  IconImage,
  IconPalette,
  IconPin,
  IconPlus,
  IconTag,
  IconText,
  IconTrash,
  IconUnarchive,
} from "./icons";

interface SidebarProps {
  view: SidebarView;
  onSelect: (view: SidebarView) => void;
  stats: Stats | null;
  collections: Collection[];
  tags: Tag[];
  onCreateCollection: (name: string) => void;
  onDeleteCollection: (collection: Collection) => void;
  onArchiveCollection: (collection: Collection, archived: boolean) => void;
  onContextMenuCollection: (collection: Collection, event: ReactMouseEvent) => void;
}

function NavItem({
  icon,
  label,
  count,
  active,
  onClick,
  title,
  hideCountOnHover,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
  title?: string;
  /** Makes room for the wrapper's hover actions instead of overlapping them. */
  hideCountOnHover?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title ?? label}
      aria-current={active ? "true" : undefined}
      className={`flex w-full items-center gap-2 rounded px-2 py-[5px] text-left text-[12px] transition-colors ${
        active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      <span className={`shrink-0 ${active ? "text-accent" : "text-ink-faint"}`}>{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {count !== undefined && (
        <span
          className={`shrink-0 text-[10px] tabular-nums text-ink-faint ${
            hideCountOnHover ? "group-hover:invisible" : ""
          }`}
        >
          {count}
        </span>
      )}
    </button>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mt-3">
      <p className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
        {title}
      </p>
      <div className="space-y-0.5">{children}</div>
    </div>
  );
}

export function Sidebar({
  view,
  onSelect,
  stats,
  collections,
  tags,
  onCreateCollection,
  onDeleteCollection,
  onArchiveCollection,
  onContextMenuCollection,
}: SidebarProps) {
  const [draftName, setDraftName] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<Collection | null>(null);

  // Images and files are counted directly; everything else is text. Cheaper than
  // three extra queries for a sidebar badge.
  const textCount = stats ? Math.max(0, stats.total - stats.images - stats.files) : undefined;
  const activeCollections = collections.filter((collection) => !collection.archived);
  const archivedCollections = collections.filter((collection) => collection.archived);

  const submitDraft = () => {
    const name = (draftName ?? "").trim();
    if (name) onCreateCollection(name);
    setDraftName(null);
  };

  // Escape must dismiss the confirmation, not fall through to the window's
  // handler, which would hide the whole app underneath it.
  useEffect(() => {
    if (!pendingDelete) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setPendingDelete(null);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [pendingDelete]);

  const collectionRow = (collection: Collection) => (
    <div
      key={collection.id}
      className="group relative"
      onContextMenu={(event) => onContextMenuCollection(collection, event)}
    >
      <NavItem
        icon={
          collection.archived ? (
            <IconArchive className="size-3.5" />
          ) : (
            <IconFolder className="size-3.5" />
          )
        }
        label={collection.name}
        count={collection.clipCount}
        hideCountOnHover
        active={view.kind === "collection" && view.id === collection.id}
        onClick={() =>
          onSelect({
            kind: "collection",
            id: collection.id,
            name: collection.name,
          })
        }
      />
      <div className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 rounded bg-panel-2 pl-1 group-hover:flex group-focus-within:flex">
        <button
          type="button"
          title={
            collection.archived
              ? `Move "${collection.name}" back into history`
              : `Archive "${collection.name}" so Clear history leaves it alone`
          }
          aria-label={
            collection.archived
              ? `Unarchive the ${collection.name} collection`
              : `Archive the ${collection.name} collection`
          }
          onClick={() => onArchiveCollection(collection, !collection.archived)}
          className="rounded p-1 text-ink-faint hover:bg-panel-3 hover:text-ink"
        >
          {collection.archived ? (
            <IconUnarchive className="size-3" />
          ) : (
            <IconArchive className="size-3" />
          )}
        </button>
        <button
          type="button"
          title={`Delete the "${collection.name}" collection`}
          aria-label={`Delete the ${collection.name} collection`}
          onClick={() => setPendingDelete(collection)}
          className="rounded p-1 text-ink-faint hover:bg-danger/20 hover:text-danger"
        >
          <IconTrash className="size-3" />
        </button>
      </div>
    </div>
  );

  return (
    <>
      <nav
        aria-label="Filters"
        className="flex w-[186px] shrink-0 flex-col overflow-y-auto border-r border-edge bg-panel px-1.5 py-2"
      >
      <Section title="History">
        <NavItem
          icon={<IconAll className="size-3.5" />}
          label="All clips"
          count={stats?.total}
          active={view.kind === "all"}
          onClick={() => onSelect({ kind: "all" })}
        />
        <NavItem
          icon={<IconText className="size-3.5" />}
          label="Text"
          count={textCount}
          active={view.kind === "text"}
          onClick={() => onSelect({ kind: "text" })}
        />
        <NavItem
          icon={<IconImage className="size-3.5" />}
          label="Images"
          count={stats?.images}
          active={view.kind === "image"}
          onClick={() => onSelect({ kind: "image" })}
        />
        <NavItem
          icon={<IconFiles className="size-3.5" />}
          label="Files"
          count={stats?.files}
          active={view.kind === "files"}
          onClick={() => onSelect({ kind: "files" })}
        />
        <NavItem
          icon={<IconPalette className="size-3.5" />}
          label="Colours"
          count={stats?.colors}
          active={view.kind === "colors"}
          onClick={() => onSelect({ kind: "colors" })}
        />
        <NavItem
          icon={<IconPin className="size-3.5" />}
          label="Pinned"
          count={stats?.pinned}
          active={view.kind === "pinned"}
          onClick={() => onSelect({ kind: "pinned" })}
        />
      </Section>

      <Section title="Collections">
        {activeCollections.map(collectionRow)}

        {draftName === null ? (
          <button
            type="button"
            onClick={() => setDraftName("")}
            className="flex w-full items-center gap-2 rounded px-2 py-[5px] text-left text-[11px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
          >
            <IconPlus className="size-3" />
            New collection
          </button>
        ) : (
          <input
            autoFocus
            value={draftName}
            onChange={(event) => setDraftName(event.target.value)}
            onBlur={submitDraft}
            onKeyDown={(event) => {
              if (event.key === "Enter") submitDraft();
              if (event.key === "Escape") setDraftName(null);
              event.stopPropagation();
            }}
            placeholder="Collection name"
            aria-label="New collection name"
            className="mx-1 w-[calc(100%-0.5rem)] rounded border border-accent-dim bg-panel-2 px-2 py-1 text-[11px] text-ink placeholder:text-ink-faint focus:outline-none"
          />
        )}
      </Section>

      {archivedCollections.length > 0 && (
        <Section title="Archives">
          {archivedCollections.map(collectionRow)}
        </Section>
      )}

      {tags.length > 0 && (
        <Section title="Tags">
          {tags.map((tag) => (
            <NavItem
              key={tag.id}
              icon={<IconTag className="size-3.5" />}
              label={tag.name}
              count={tag.clipCount}
              active={view.kind === "tag" && view.name === tag.name}
              onClick={() => onSelect({ kind: "tag", name: tag.name })}
            />
          ))}
        </Section>
      )}
      </nav>

      {pendingDelete && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 p-6 animate-fade-in"
          onMouseDown={() => setPendingDelete(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-collection-title"
            onMouseDown={(event) => event.stopPropagation()}
            className="w-full max-w-[330px] rounded-panel border border-edge bg-panel p-4 shadow-xl animate-rise"
          >
            <h2 id="delete-collection-title" className="text-[13px] font-semibold text-ink">
              Delete “{pendingDelete.name}”?
            </h2>
            <p className="mt-1.5 text-[11px] leading-relaxed text-ink-dim">
              {pendingDelete.archived
                ? "The collection goes away and its clips return to your history. The clips themselves are not deleted."
                : "Only the grouping is removed — the clips stay in your history."}
            </p>
            <div className="mt-3 flex justify-end gap-2">
              <button
                type="button"
                autoFocus
                onClick={() => setPendingDelete(null)}
                className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  onDeleteCollection(pendingDelete);
                  setPendingDelete(null);
                }}
                className="rounded border border-danger bg-danger px-2.5 py-1 text-[11px] text-white transition-colors hover:bg-danger/90"
              >
                Delete collection
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
