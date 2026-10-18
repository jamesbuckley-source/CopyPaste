import { useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";

import { EMOJI_GROUPS, emojiByChar, searchEmoji, type EmojiEntry } from "../lib/emoji";
import { IconSearch, IconStar } from "./icons";

interface EmojiBrowserProps {
  /** Chars pinned to the top of the browser. */
  favorites: string[];
  onCopy: (entry: EmojiEntry) => void;
  onContextMenu: (entry: EmojiEntry, event: ReactMouseEvent, isFavorite: boolean) => void;
}

function EmojiButton({
  entry,
  favorite,
  onCopy,
  onContextMenu,
}: {
  entry: EmojiEntry;
  favorite: boolean;
  onCopy: () => void;
  onContextMenu: (event: ReactMouseEvent) => void;
}) {
  return (
    <button
      type="button"
      onClick={onCopy}
      onContextMenu={onContextMenu}
      title={`${entry.name} — click to copy`}
      aria-label={`${entry.name} emoji`}
      className={`group relative flex aspect-square items-center justify-center rounded border border-transparent text-[22px] leading-none transition-colors hover:border-edge hover:bg-panel-3 ${
        favorite ? "bg-accent/10" : ""
      }`}
    >
      <span aria-hidden="true">{entry.char}</span>
      {favorite && (
        <span className="absolute right-0.5 top-0.5 text-warn">
          <IconStar className="size-2.5" filled />
        </span>
      )}
    </button>
  );
}

export function EmojiBrowser({ favorites, onCopy, onContextMenu }: EmojiBrowserProps) {
  const [query, setQuery] = useState("");
  const results = useMemo(() => searchEmoji(query), [query]);
  const searching = query.trim().length > 0;

  const favouriteEntries = favorites
    .map((char) => emojiByChar(char))
    .filter((entry): entry is EmojiEntry => entry !== undefined);

  const renderGrid = (entries: EmojiEntry[]) => (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(38px,1fr))] gap-1">
      {entries.map((entry) => (
        <EmojiButton
          key={`${entry.char}-${entry.name}`}
          entry={entry}
          favorite={favorites.includes(entry.char)}
          onCopy={() => onCopy(entry)}
          onContextMenu={(event) => onContextMenu(entry, event, favorites.includes(entry.char))}
        />
      ))}
    </div>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-edge bg-panel px-3 py-2">
        <IconSearch className="size-3.5 shrink-0 text-ink-faint" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          autoFocus
          spellCheck={false}
          autoComplete="off"
          placeholder="Search emoji…  (fire, rocket, thank you)"
          aria-label="Search emoji"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-faint focus:outline-none"
        />
        <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">
          {searching ? `${results.length} match${results.length === 1 ? "" : "es"}` : ""}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
        {!searching && favouriteEntries.length > 0 && (
          <section className="mb-3">
            <h3 className="pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
              Favourites
            </h3>
            {renderGrid(favouriteEntries)}
          </section>
        )}

        {searching ? (
          results.length === 0 ? (
            <p className="py-8 text-center text-[11px] text-ink-faint">
              No emoji matches “{query.trim()}”.
            </p>
          ) : (
            renderGrid(results)
          )
        ) : (
          EMOJI_GROUPS.map((group) => (
            <section key={group.title} className="mb-3">
              <h3 className="pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
                {group.title}
              </h3>
              {renderGrid(group.entries)}
            </section>
          ))
        )}

        <p className="pb-2 text-[10px] leading-relaxed text-ink-faint">
          Click an emoji to copy it. Right-click one to pin it to Favourites; right-click a
          favourite to remove it (it stays in the library).
        </p>
      </div>
    </div>
  );
}
