import { useEffect, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";

import type { Collection, SavedSearch, SidebarView, Stats, Tag, TextContentKind } from "../lib/types";
import { TEXT_CONTENT_LABEL } from "../lib/types";
import {
  IconAll,
  IconArchive,
  IconChevronRight,
  IconEmoji,
  IconFiles,
  IconFolder,
  IconGif,
  IconImage,
  IconPalette,
  IconPin,
  IconPlus,
  IconSearch,
  IconShield,
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
  savedSearches: SavedSearch[];
  onCreateCollection: (name: string) => void;
  onCreateArchive: (name: string) => void;
  onDeleteCollection: (collection: Collection) => void;
  onArchiveCollection: (collection: Collection, archived: boolean) => void;
  onContextMenuCollection: (collection: Collection, event: ReactMouseEvent) => void;
  onContextMenuTag: (tag: Tag, event: ReactMouseEvent) => void;
  onContextMenuSearch: (search: SavedSearch, event: ReactMouseEvent) => void;
  /** How many collections/archives appear before "Show all". */
  listLimit: number;
  /** Files clips dropped from the history onto a collection or archive. */
  onDropClips: (ids: number[], collection: Collection) => void;
  /** Whether the text payload sub-types are expanded. */
  textExpanded: boolean;
  onToggleText: () => void;
  /** Sections the user collapsed; History is never collapsible. */
  collapsedSections: string[];
  onToggleSection: (name: string) => void;
}

/** Tags stop at this many entries before the list offers "Show all". */
const TAG_LIMIT = 10;
/** Custom drag payload carrying the ids of the clips being dragged. */
const CLIP_DRAG_TYPE = "application/x-copypaste-clips";
/** Expanded archive/tag lists stay compact and scroll instead of pushing
 * utilities off the screen. */
const EXPANDED_LIST_CLASS = "max-h-56 overflow-y-auto scrollbar-none";

const TEXT_SUBVIEWS: TextContentKind[] = ["url", "email", "path", "json", "markdown"];

function NavItem({
  icon,
  label,
  count,
  active,
  onClick,
  title,
  hideCountOnHover,
  indent,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
  title?: string;
  /** Makes room for the wrapper's hover actions instead of overlapping them. */
  hideCountOnHover?: boolean;
  /** Nests the row one level under its parent. */
  indent?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title ?? label}
      aria-current={active ? "true" : undefined}
      className={`flex w-full items-center gap-2 rounded py-[5px] pr-2 text-left text-[12px] transition-colors ${
        indent ? "pl-6" : "pl-2"
      } ${active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"}`}
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

/** A nav row with a separate chevron control for an inline list of children. */
function ExpandableNavItem({
  icon,
  label,
  count,
  active,
  expanded,
  onClick,
  onToggle,
  title,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  expanded: boolean;
  onClick: () => void;
  onToggle: () => void;
  title?: string;
}) {
  return (
    <div
      onDoubleClick={onToggle}
      title={`Double-click to ${expanded ? "collapse" : "expand"} ${label.toLowerCase()}`}
      className={`flex w-full items-center rounded transition-colors ${
        active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      <button
        type="button"
        onClick={onClick}
        title={title ?? label}
        aria-current={active ? "true" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2 py-[5px] pl-2 text-left text-[12px]"
      >
        <span className={`shrink-0 ${active ? "text-accent" : "text-ink-faint"}`}>{icon}</span>
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {count !== undefined && (
          <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">{count}</span>
        )}
      </button>
      <button
        type="button"
        onClick={onToggle}
        onDoubleClick={(event) => event.stopPropagation()}
        aria-expanded={expanded}
        aria-label={`${expanded ? "Collapse" : "Expand"} text types`}
        title={`${expanded ? "Collapse" : "Expand"} text types`}
        className="shrink-0 rounded p-1 text-ink-faint transition-colors hover:text-ink"
      >
        <IconChevronRight
          className={`size-3 transition-transform ${expanded ? "rotate-90" : ""}`}
        />
      </button>
    </div>
  );
}

function Section({
  title,
  children,
  collapsible = false,
  collapsed = false,
  onToggle,
}: {
  title: string;
  children: ReactNode;
  /** Sections other than History can fold away to shorten the sidebar. */
  collapsible?: boolean;
  collapsed?: boolean;
  onToggle?: () => void;
}) {
  return (
    <div className="mt-3">
      {collapsible ? (
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={!collapsed}
          title={`${collapsed ? "Expand" : "Collapse"} ${title.toLowerCase()}`}
          className="flex w-full items-center gap-1 border-y border-edge-soft px-2 py-1 text-left"
        >
          <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
            {title}
          </span>
          <IconChevronRight
            className={`ml-auto size-3 shrink-0 text-ink-faint transition-transform ${
              collapsed ? "" : "rotate-90"
            }`}
          />
        </button>
      ) : (
        <p className="border-y border-edge-soft px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
          {title}
        </p>
      )}
      {!collapsed && <div className="space-y-0.5">{children}</div>}
    </div>
  );
}

export function Sidebar({
  view,
  onSelect,
  stats,
  collections,
  tags,
  savedSearches,
  onCreateCollection,
  onCreateArchive,
  onDeleteCollection,
  onArchiveCollection,
  onContextMenuCollection,
  onContextMenuTag,
  onContextMenuSearch,
  listLimit,
  onDropClips,
  textExpanded,
  onToggleText,
  collapsedSections,
  onToggleSection,
}: SidebarProps) {
  const [draftName, setDraftName] = useState<string | null>(null);
  const [draftArchive, setDraftArchive] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<Collection | null>(null);
  const [tagFilter, setTagFilter] = useState("");
  const [archiveFilter, setArchiveFilter] = useState("");
  const [collectionFilter, setCollectionFilter] = useState("");
  const [showAllArchives, setShowAllArchives] = useState(false);
  const [showAllCollections, setShowAllCollections] = useState(false);
  const [showAllTags, setShowAllTags] = useState(false);
  /** Collection currently highlighted as a drop target. */
  const [dropId, setDropId] = useState<number | null>(null);

  const foldable = (name: string) => ({
    collapsible: true,
    collapsed: collapsedSections.includes(name),
    onToggle: () => onToggleSection(name),
  });

  // Images and files are counted directly; everything else is text. Cheaper than
  // three extra queries for a sidebar badge.
  const textCount = stats ? Math.max(0, stats.total - stats.images - stats.files) : undefined;
  const activeCollections = collections.filter((collection) => !collection.archived);
  const archivedCollections = collections.filter((collection) => collection.archived);
  const tagQuery = tagFilter.trim().toLowerCase();
  const matchingTags = tagQuery
    ? tags.filter((tag) => tag.name.toLowerCase().includes(tagQuery))
    : tags;
  const visibleTags = showAllTags ? matchingTags : matchingTags.slice(0, TAG_LIMIT);
  const archiveQuery = archiveFilter.trim().toLowerCase();
  const matchingArchives = archiveQuery
    ? archivedCollections.filter((collection) => collection.name.toLowerCase().includes(archiveQuery))
    : archivedCollections;
  const visibleArchives = showAllArchives
    ? matchingArchives
    : matchingArchives.slice(0, Math.max(1, listLimit));
  const collectionQuery = collectionFilter.trim().toLowerCase();
  const matchingCollections = collectionQuery
    ? activeCollections.filter((collection) =>
        collection.name.toLowerCase().includes(collectionQuery),
      )
    : activeCollections;
  const visibleCollections = showAllCollections
    ? matchingCollections
    : matchingCollections.slice(0, Math.max(1, listLimit));

  const submitDraft = () => {
    const name = (draftName ?? "").trim();
    if (name) onCreateCollection(name);
    setDraftName(null);
  };

  const submitArchiveDraft = () => {
    const name = (draftArchive ?? "").trim();
    if (name) onCreateArchive(name);
    setDraftArchive(null);
  };

  // A drag that ends anywhere else must release the drop highlight.
  useEffect(() => {
    const clear = () => setDropId(null);
    window.addEventListener("dragend", clear);
    return () => window.removeEventListener("dragend", clear);
  }, []);

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
      onContextMenu={(event) => onContextMenuCollection(collection, event)}
      onDragOver={(event) => {
        if (!Array.from(event.dataTransfer.types).includes(CLIP_DRAG_TYPE)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
        setDropId(collection.id);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setDropId((current) => (current === collection.id ? null : current));
        }
      }}
      onDrop={(event) => {
        if (!Array.from(event.dataTransfer.types).includes(CLIP_DRAG_TYPE)) return;
        event.preventDefault();
        setDropId(null);
        try {
          const parsed: unknown = JSON.parse(event.dataTransfer.getData(CLIP_DRAG_TYPE));
          if (!Array.isArray(parsed)) return;
          const ids = parsed.filter((id): id is number => typeof id === "number");
          if (ids.length > 0) onDropClips(ids, collection);
        } catch {
          // A foreign payload is not ours to interpret.
        }
      }}
      className={`group relative rounded transition-colors ${
        dropId === collection.id ? "bg-accent/15 ring-1 ring-accent" : ""
      }`}
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
          icon={<IconPin className="size-3.5" />}
          label="Pinned"
          count={stats?.pinned}
          active={view.kind === "pinned"}
          onClick={() => onSelect({ kind: "pinned" })}
        />
        <ExpandableNavItem
          icon={<IconText className="size-3.5" />}
          label="Text"
          count={textCount}
          active={
            view.kind === "text" || view.kind === "content" || view.kind === "secrets"
          }
          expanded={textExpanded}
          onClick={() => onSelect({ kind: "text" })}
          onToggle={onToggleText}
        />
        {textExpanded && (
          <>
            {TEXT_SUBVIEWS.map((contentKind) => (
              <NavItem
                key={contentKind}
                indent
                icon={
                  <span
                    aria-hidden="true"
                    className="block size-1.5 rounded-full border border-current text-ink-faint"
                  />
                }
                label={TEXT_CONTENT_LABEL[contentKind]}
                title={`Text clips that are a ${TEXT_CONTENT_LABEL[contentKind].toLowerCase()}`}
                active={view.kind === "content" && view.contentKind === contentKind}
                onClick={() => onSelect({ kind: "content", contentKind })}
              />
            ))}
            <NavItem
              indent
              icon={<IconShield className="size-3" />}
              label="Secrets"
              title="Clips flagged as credential-like"
              active={view.kind === "secrets"}
              onClick={() => onSelect({ kind: "secrets" })}
            />
          </>
        )}
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
          label="Colors"
          count={stats?.colors}
          active={view.kind === "colors"}
          onClick={() => onSelect({ kind: "colors" })}
        />
      </Section>

      <Section title="Collections" {...foldable("collections")}>
        {activeCollections.length > 0 && (
          <div className="relative px-1 pb-1">
            <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
            <input
              value={collectionFilter}
              onChange={(event) => setCollectionFilter(event.target.value)}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === "Escape") {
                  event.preventDefault();
                  setCollectionFilter("");
                }
                if (event.key === "Enter" && matchingCollections.length > 0) {
                  event.preventDefault();
                  const first = matchingCollections[0];
                  onSelect({ kind: "collection", id: first.id, name: first.name });
                }
              }}
              placeholder="Filter collections"
              aria-label="Filter collections"
              spellCheck={false}
              autoComplete="off"
              className="w-full rounded border border-edge bg-panel-2 py-[3px] pl-6 pr-1.5 text-[10px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
            />
          </div>
        )}
        <div className={showAllCollections ? EXPANDED_LIST_CLASS : undefined}>
          {visibleCollections.map(collectionRow)}
        </div>
        {matchingCollections.length === 0 && activeCollections.length > 0 && (
          <p className="px-2 pb-1 text-[10px] text-ink-faint">No collection matches.</p>
        )}
        {matchingCollections.length > listLimit && (
          <button
            type="button"
            onClick={() => setShowAllCollections((shown) => !shown)}
            className="w-full rounded px-2 py-[5px] text-left text-[11px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
          >
            {showAllCollections ? "Show fewer" : `Show all ${matchingCollections.length}`}
          </button>
        )}

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

      <Section title="Archives" {...foldable("archives")}>
        {archivedCollections.length > 0 && (
          <div className="relative px-1 pb-1">
            <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
            <input
              value={archiveFilter}
              onChange={(event) => setArchiveFilter(event.target.value)}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === "Escape") {
                  event.preventDefault();
                  setArchiveFilter("");
                }
                if (event.key === "Enter" && matchingArchives.length > 0) {
                  event.preventDefault();
                  const first = matchingArchives[0];
                  onSelect({ kind: "collection", id: first.id, name: first.name });
                }
              }}
              placeholder="Filter archives"
              aria-label="Filter archives"
              spellCheck={false}
              autoComplete="off"
              className="w-full rounded border border-edge bg-panel-2 py-[3px] pl-6 pr-1.5 text-[10px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
            />
          </div>
        )}
        <div className={showAllArchives ? EXPANDED_LIST_CLASS : undefined}>
          {visibleArchives.map(collectionRow)}
        </div>
        {matchingArchives.length === 0 && archivedCollections.length > 0 && (
          <p className="px-2 pb-1 text-[10px] text-ink-faint">No archive matches.</p>
        )}
        {matchingArchives.length > listLimit && (
          <button
            type="button"
            onClick={() => setShowAllArchives((shown) => !shown)}
            className="w-full rounded px-2 py-[5px] text-left text-[11px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
          >
            {showAllArchives
              ? "Show fewer"
              : `Show all ${matchingArchives.length}`}
          </button>
        )}

        {draftArchive === null ? (
          <button
            type="button"
            onClick={() => setDraftArchive("")}
            className="flex w-full items-center gap-2 rounded px-2 py-[5px] text-left text-[11px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
          >
            <IconPlus className="size-3" />
            New archive
          </button>
        ) : (
          <input
            autoFocus
            value={draftArchive}
            onChange={(event) => setDraftArchive(event.target.value)}
            onBlur={submitArchiveDraft}
            onKeyDown={(event) => {
              if (event.key === "Enter") submitArchiveDraft();
              if (event.key === "Escape") setDraftArchive(null);
              event.stopPropagation();
            }}
            placeholder="Archive name"
            aria-label="New archive name"
            className="mx-1 w-[calc(100%-0.5rem)] rounded border border-accent-dim bg-panel-2 px-2 py-1 text-[11px] text-ink placeholder:text-ink-faint focus:outline-none"
          />
        )}
      </Section>

      {savedSearches.length > 0 && (
        <Section title="Saved searches">
          {savedSearches.map((search) => (
            <div
              key={search.name}
              className="group relative"
              onContextMenu={(event) => onContextMenuSearch(search, event)}
            >
              <NavItem
                icon={<IconSearch className="size-3.5" />}
                label={search.name}
                title={`${search.name} — ${search.query}`}
                active={view.kind === "search" && view.name === search.name}
                onClick={() => onSelect({ kind: "search", name: search.name, query: search.query })}
              />
            </div>
          ))}
        </Section>
      )}

      {tags.length > 0 && (
        <Section title="Tags" {...foldable("tags")}>
          <div className="relative px-1 pb-1">
            <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
            <input
              value={tagFilter}
              onChange={(event) => setTagFilter(event.target.value)}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === "Escape") {
                  event.preventDefault();
                  setTagFilter("");
                }
                if (event.key === "Enter" && matchingTags.length > 0) {
                  event.preventDefault();
                  const first = matchingTags[0];
                  onSelect({ kind: "tag", name: first.name });
                }
              }}
              placeholder="Filter tags"
              aria-label="Filter tags"
              spellCheck={false}
              autoComplete="off"
              className="w-full rounded border border-edge bg-panel-2 py-[3px] pl-6 pr-1.5 text-[10px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
            />
          </div>
          <div className={showAllTags ? EXPANDED_LIST_CLASS : undefined}>
            {visibleTags.map((tag) => (
              <div
                key={tag.id}
                className="group relative"
                onContextMenu={(event) => onContextMenuTag(tag, event)}
              >
                <NavItem
                  icon={
                    tag.color ? (
                      <span
                        aria-hidden="true"
                        style={{ backgroundColor: tag.color }}
                        className="block size-2.5 rounded-full border border-edge"
                      />
                    ) : (
                      <IconTag className="size-3.5" />
                    )
                  }
                  label={tag.name}
                  title={`#${tag.name} — right-click to rename, recolor or delete`}
                  count={tag.clipCount}
                  active={view.kind === "tag" && view.name === tag.name}
                  onClick={() => onSelect({ kind: "tag", name: tag.name })}
                />
              </div>
            ))}
          </div>
          {matchingTags.length === 0 && (
            <p className="px-2 pb-1 text-[10px] text-ink-faint">No tag matches.</p>
          )}
          {matchingTags.length > TAG_LIMIT && (
            <button
              type="button"
              onClick={() => setShowAllTags((shown) => !shown)}
              className="w-full rounded px-2 py-[5px] text-left text-[11px] text-ink-faint transition-colors hover:bg-panel-2 hover:text-ink-dim"
            >
              {showAllTags ? "Show fewer" : `Show all ${matchingTags.length}`}
            </button>
          )}
        </Section>
      )}

      <Section title="Utilities" {...foldable("utilities")}>
        <NavItem
          icon={<IconEmoji className="size-3.5" />}
          label="Emoji"
          title="Browse and copy emoji, with favourites"
          active={view.kind === "emoji"}
          onClick={() => onSelect({ kind: "emoji" })}
        />
        <NavItem
          icon={<IconGif className="size-3.5" />}
          label="GIFs"
          title="Search GIPHY for animated GIFs to copy"
          active={view.kind === "gifs"}
          onClick={() => onSelect({ kind: "gifs" })}
        />
      </Section>
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
