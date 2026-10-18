import { useState, type ReactNode } from "react";

import type { Collection, SidebarView, Stats, Tag } from "../lib/types";
import {
  IconAll,
  IconFiles,
  IconFolder,
  IconImage,
  IconPin,
  IconPlus,
  IconTag,
  IconText,
  IconTrash,
} from "./icons";

interface SidebarProps {
  view: SidebarView;
  onSelect: (view: SidebarView) => void;
  stats: Stats | null;
  collections: Collection[];
  tags: Tag[];
  onCreateCollection: (name: string) => void;
  onDeleteCollection: (collection: Collection) => void;
}

function NavItem({
  icon,
  label,
  count,
  active,
  onClick,
  title,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
  title?: string;
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
        <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">{count}</span>
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
}: SidebarProps) {
  const [draftName, setDraftName] = useState<string | null>(null);

  // Images and files are counted directly; everything else is text. Cheaper than
  // three extra queries for a sidebar badge.
  const textCount = stats ? Math.max(0, stats.total - stats.images - stats.files) : undefined;

  const submitDraft = () => {
    const name = (draftName ?? "").trim();
    if (name) onCreateCollection(name);
    setDraftName(null);
  };

  return (
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
          icon={<IconPin className="size-3.5" />}
          label="Pinned"
          count={stats?.pinned}
          active={view.kind === "pinned"}
          onClick={() => onSelect({ kind: "pinned" })}
        />
      </Section>

      <Section title="Collections">
        {collections.map((collection) => (
          <div key={collection.id} className="group relative">
            <NavItem
              icon={<IconFolder className="size-3.5" />}
              label={collection.name}
              count={collection.clipCount}
              active={view.kind === "collection" && view.id === collection.id}
              onClick={() =>
                onSelect({ kind: "collection", id: collection.id, name: collection.name })
              }
            />
            <button
              type="button"
              title={`Delete the "${collection.name}" collection`}
              aria-label={`Delete the ${collection.name} collection`}
              onClick={() => onDeleteCollection(collection)}
              className="absolute right-1 top-1/2 hidden -translate-y-1/2 rounded p-1 text-ink-faint hover:bg-danger/20 hover:text-danger group-hover:block"
            >
              <IconTrash className="size-3" />
            </button>
          </div>
        ))}

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
  );
}
