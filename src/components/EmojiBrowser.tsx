import { memo, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type UIEvent as ReactUIEvent } from "react";

import { EMOJI_GROUPS, ALL_EMOJI, emojiByChar, searchEmoji, type EmojiEntry } from "../lib/emoji";
import { warmEmojiGlyphs } from "../lib/emojiWarm";
import { IconSearch, IconStar } from "./icons";

interface EmojiBrowserProps {
  /** Chars pinned to the top of the browser. */
  favorites: string[];
  onCopy: (entry: EmojiEntry) => void;
  onContextMenu: (entry: EmojiEntry, event: ReactMouseEvent, isFavorite: boolean) => void;
}

/** Cell geometry, in CSS pixels. Rows are laid out by hand so they can be virtualised. */
const CELL = 38;
const GAP = 4;
const HEADING_HEIGHT = 24;
const NOTE_HEIGHT = 44;
/** Rows rendered beyond the viewport, so a fast flick does not reveal a gap. */
const OVERSCAN = 4;

type Row =
  | { type: "heading"; title: string; height: number; key: string }
  | { type: "emojis"; entries: EmojiEntry[]; height: number; key: string }
  | { type: "note"; text: string; height: number; key: string }
  | { type: "message"; text: string; height: number; key: string };

const EmojiButton = memo(function EmojiButton({
  entry,
  favorite,
  onCopy,
  onContextMenu,
}: {
  entry: EmojiEntry;
  favorite: boolean;
  onCopy: (entry: EmojiEntry) => void;
  onContextMenu: (entry: EmojiEntry, event: ReactMouseEvent, isFavorite: boolean) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onCopy(entry)}
      onContextMenu={(event) => onContextMenu(entry, event, favorite)}
      title={`${entry.name} — click to copy`}
      aria-label={`${entry.name} emoji`}
      className={`relative flex items-center justify-center rounded border border-transparent text-[22px] leading-none transition-colors hover:border-edge hover:bg-panel-3 ${
        favorite ? "bg-accent/10" : ""
      }`}
      style={{ height: CELL }}
    >
      <span aria-hidden="true">{entry.char}</span>
      {favorite && (
        <span className="absolute right-0.5 top-0.5 text-warn">
          <IconStar className="size-2.5" filled />
        </span>
      )}
    </button>
  );
});

/** Index of the row containing `offset`, using the precomputed row tops. */
function rowAt(offsets: number[], offset: number): number {
  let low = 0;
  let high = Math.max(0, offsets.length - 1);
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (offsets[middle] <= offset) low = middle;
    else high = middle - 1;
  }
  return low;
}

export const EmojiBrowser = memo(function EmojiBrowser({
  favorites,
  onCopy,
  onContextMenu,
}: EmojiBrowserProps) {
  const [query, setQuery] = useState("");
  // Opening on one category keeps the first paint to ~100 glyphs instead of the
  // whole library; "All" is still one click away.
  const [category, setCategory] = useState<string>(EMOJI_GROUPS[0].title);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const scrollFrame = useRef(0);
  const latestScroll = useRef(0);

  // Scroll events can fire more than once per frame. Coalescing them keeps the
  // virtual window recomputation to one render per painted frame.
  const handleScroll = (event: ReactUIEvent<HTMLDivElement>) => {
    latestScroll.current = event.currentTarget.scrollTop;
    if (scrollFrame.current) return;
    scrollFrame.current = window.requestAnimationFrame(() => {
      scrollFrame.current = 0;
      setScrollTop(latestScroll.current);
    });
  };

  const resetScroll = () => {
    latestScroll.current = 0;
    if (scrollFrame.current) {
      window.cancelAnimationFrame(scrollFrame.current);
      scrollFrame.current = 0;
    }
    scrollerRef.current?.scrollTo({ top: 0 });
    setScrollTop(0);
  };

  useEffect(
    () => () => {
      if (scrollFrame.current) window.cancelAnimationFrame(scrollFrame.current);
    },
    [],
  );

  const results = useMemo(() => searchEmoji(query), [query]);
  const searching = query.trim().length > 0;
  const favoriteChars = useMemo(() => new Set(favorites), [favorites]);
  const favouriteEntries = useMemo(
    () =>
      favorites
        .map((char) => emojiByChar(char))
        .filter((entry): entry is EmojiEntry => entry !== undefined),
    [favorites],
  );

  // The first paint of a row is cheap; the stall happens when the compositor
  // rasterises emoji glyphs for the first time as they scroll into view. Warm
  // the glyph cache for the section on screen while the view is idle.
  const visibleEntries = useMemo(() => {
    if (searching) return results;
    if (category === "all") return ALL_EMOJI;
    return EMOJI_GROUPS.find((group) => group.title === category)?.entries ?? [];
  }, [category, results, searching]);

  useEffect(() => warmEmojiGlyphs(visibleEntries), [visibleEntries]);

  // The scroller owns its own size, so the visible window can be computed without
  // a layout pass on every keystroke.
  useEffect(() => {
    const element = scrollerRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      setViewport({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(element);
    setViewport({ width: element.clientWidth, height: element.clientHeight });
    return () => observer.disconnect();
  }, []);

  const columns = Math.max(1, Math.floor((viewport.width + GAP) / (CELL + GAP)));

  // Laying every emoji out as a row keeps the DOM to roughly a screenful: the
  // first paint used to build all ~535 buttons and rasterise every glyph at once.
  const { rows, offsets, totalHeight } = useMemo(() => {
    const built: Row[] = [];
    const pack = (list: Row[]) => {
      const tops: number[] = new Array(list.length);
      let total = 0;
      for (let index = 0; index < list.length; index += 1) {
        tops[index] = total;
        total += list[index].height;
      }
      return { rows: list, offsets: tops, totalHeight: total };
    };
    const pushEmojis = (entries: EmojiEntry[], keyPrefix: string) => {
      for (let index = 0; index < entries.length; index += columns) {
        built.push({
          type: "emojis",
          entries: entries.slice(index, index + columns),
          height: CELL + GAP,
          key: `${keyPrefix}-${index}`,
        });
      }
    };

    if (searching) {
      pushEmojis(results, "results");
      if (results.length === 0) {
        built.push({
          type: "message",
          text: `No emoji matches “${query.trim()}”.`,
          height: NOTE_HEIGHT,
          key: "empty",
        });
      }
      return pack(built);
    }

    if (favouriteEntries.length > 0) {
      built.push({ type: "heading", title: "Favourites", height: HEADING_HEIGHT, key: "fav" });
      pushEmojis(favouriteEntries, "fav");
    }
    for (const emojiGroup of EMOJI_GROUPS) {
      if (category !== "all" && emojiGroup.title !== category) continue;
      if (category === "all") {
        built.push({
          type: "heading",
          title: emojiGroup.title,
          height: HEADING_HEIGHT,
          key: `h-${emojiGroup.title}`,
        });
      }
      pushEmojis(emojiGroup.entries, emojiGroup.title);
    }
    built.push({
      type: "note",
      text: "Click an emoji to copy it. Right-click one to pin it to Favourites; right-click a favourite to remove it (it stays in the library).",
      height: NOTE_HEIGHT,
      key: "note",
    });
    return pack(built);
  }, [category, columns, favouriteEntries, query, results, searching]);

  const visible = useMemo(() => {
    if (rows.length === 0) return [] as { row: Row; top: number }[];
    const first = Math.max(0, rowAt(offsets, scrollTop) - OVERSCAN);
    const last = Math.min(
      rows.length - 1,
      rowAt(offsets, scrollTop + viewport.height) + OVERSCAN,
    );
    const out: { row: Row; top: number }[] = [];
    for (let index = first; index <= last; index += 1) {
      out.push({ row: rows[index], top: offsets[index] });
    }
    return out;
  }, [rows, offsets, scrollTop, viewport.height]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-edge bg-panel px-3 py-2">
        <IconSearch className="size-3.5 shrink-0 text-ink-faint" />
        <input
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            // A new result set starts at the top again.
            resetScroll();
          }}
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

      <div className="flex shrink-0 gap-1 overflow-x-auto border-b border-edge bg-panel px-3 pb-1.5">
        {[{ title: "all", label: "All" }, ...EMOJI_GROUPS.map((group) => ({ title: group.title, label: group.title }))].map(
          (chip) => (
            <button
              key={chip.title}
              type="button"
              onClick={() => {
                setCategory(chip.title);
                resetScroll();
              }}
              aria-pressed={!searching && category === chip.title}
              className={`shrink-0 rounded border px-1.5 py-0.5 text-[10px] transition-colors ${
                !searching && category === chip.title
                  ? "border-accent bg-accent/20 text-accent-hi"
                  : "border-edge bg-panel-2 text-ink-faint hover:text-ink-dim"
              }`}
            >
              {chip.label}
            </button>
          ),
        )}
      </div>

      <div
        ref={scrollerRef}
        onScroll={handleScroll}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      >
        <div style={{ height: totalHeight, position: "relative" }}>
          {visible.map(({ row, top }) => {
            if (row.type === "heading") {
              return (
                <h3
                  key={row.key}
                  style={{ position: "absolute", top, left: 0, right: 0, height: row.height }}
                  className="text-[10px] font-semibold uppercase tracking-wider text-ink-faint"
                >
                  {row.title}
                </h3>
              );
            }
            if (row.type === "note" || row.type === "message") {
              return (
                <p
                  key={row.key}
                  style={{ position: "absolute", top, left: 0, right: 0, height: row.height }}
                  className={`text-[10px] leading-relaxed text-ink-faint ${
                    row.type === "message" ? "py-6 text-center text-[11px]" : ""
                  }`}
                >
                  {row.text}
                </p>
              );
            }
            return (
              <div
                key={row.key}
                style={{
                  position: "absolute",
                  top,
                  left: 0,
                  right: 0,
                  height: row.height - GAP,
                  display: "grid",
                  gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                  gap: `${GAP}px`,
                  contain: "layout paint style",
                }}
              >
                {row.entries.map((entry) => (
                  <EmojiButton
                    key={`${entry.char}-${entry.name}`}
                    entry={entry}
                    favorite={favoriteChars.has(entry.char)}
                    onCopy={onCopy}
                    onContextMenu={onContextMenu}
                  />
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
});
