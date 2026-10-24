import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import {
  copyCustomGif,
  copyGif,
  errorMessage,
  gifFavoriteFromResult,
  getClip,
  importCustomGif,
  mediaUrl,
  searchGiphy,
  setClipSavedGif,
  type GifResult,
} from "../lib/ipc";
import type { GifFavorite } from "../lib/types";
import { AnimatedImage } from "./AnimatedImage";
import { IconClose, IconGif, IconLoop, IconPlus, IconSearch, IconStar } from "./icons";

interface GifBrowserProps {
  /** GIFs and clips pinned to the shelf at the top. */
  favorites: GifFavorite[];
  /** False shows a setup hint instead of calling GIPHY. */
  hasApiKey: boolean;
  /** Preview size multiplier, 0.6 – 2.0. */
  scale: number;
  onScaleCommit: (scale: number) => void;
  /** True keeps previews looping; off plays them only while hovered. */
  loop: boolean;
  onLoopChange: (loop: boolean) => void;
  onFavoritesChange: (favorites: GifFavorite[]) => void;
  /** Copies an image clip from the history. */
  onCopyClip: (clipId: number) => void;
  onNotice: (text: string) => void;
  onError: (text: string) => void;
}

/** Card geometry follows the same 60–200% range as the history grid. */
function geometry(scale: number) {
  const clamped = Math.min(2, Math.max(0.6, scale));
  return {
    minWidth: Math.round(150 * clamped),
    mediaHeight: Math.round(104 * clamped),
  };
}

function GifCard({
  title,
  still,
  animated,
  loop,
  width,
  height,
  favorite,
  onCopy,
  onToggleFavorite,
}: {
  title: string;
  /** Static frame, when one is available. */
  still: string | null;
  /** Animation shown on hover when looping is off. */
  animated: string | null;
  loop: boolean;
  width: number;
  height: number;
  /** `null` hides the star, used by the shelf where removal is explicit. */
  favorite: boolean | null;
  onCopy: () => void;
  onToggleFavorite?: () => void;
}) {
  return (
    <div
      className="group relative flex flex-col overflow-hidden rounded-lg border border-edge bg-panel-2 transition-colors hover:border-accent-dim hover:bg-panel-3"
      style={{ minWidth: width }}
    >
      <button
        type="button"
        onClick={onCopy}
        title={`${title || "GIF"} — click to copy`}
        className="flex flex-col text-left"
      >
        <span className="relative block shrink-0 overflow-hidden bg-canvas/70" style={{ height }}>
          <AnimatedImage
            still={still}
            animated={animated}
            loop={loop}
            alt={title}
            className="h-full w-full object-cover transition-transform duration-150 group-hover:scale-[1.02]"
          />
        </span>
        <span className="block truncate px-2 py-1 text-[10px] text-ink-dim" title={title}>
          {title || "GIF"}
        </span>
      </button>
      {favorite !== null && onToggleFavorite && (
        <button
          type="button"
          aria-label={favorite ? "Remove from My GIFs" : "Add to My GIFs"}
          aria-pressed={favorite}
          title={favorite ? "Remove from My GIFs" : "Add to My GIFs"}
          onClick={(event) => {
            event.stopPropagation();
            onToggleFavorite();
          }}
          className={`absolute right-1 top-1 rounded bg-canvas/85 p-1 transition-colors hover:text-warn ${
            favorite ? "text-warn" : "text-ink-dim"
          }`}
        >
          <IconStar className="size-3" filled={favorite} />
        </button>
      )}
    </div>
  );
}

export function GifBrowser({
  favorites,
  hasApiKey,
  scale,
  onScaleCommit,
  loop,
  onLoopChange,
  onFavoritesChange,
  onCopyClip,
  onNotice,
  onError,
}: GifBrowserProps) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<GifResult[]>([]);
  const [loading, setLoading] = useState(true);
  /** Still frame and animation per shelf clip, resolved from the history. */
  const [clipThumbs, setClipThumbs] = useState<
    Record<number, { still: string | null; animated: string | null }>
  >({});
  const [liveScale, setLiveScale] = useState<number | null>(null);

  const percent = liveScale ?? Math.round(scale * 100);
  const { minWidth, mediaHeight } = geometry(percent / 100);

  // Search as the user types, but keep the request rate sane.
  useEffect(() => {
    if (!hasApiKey) {
      setResults([]);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    const timer = window.setTimeout(() => {
      searchGiphy(query)
        .then((items) => {
          if (!cancelled) setResults(items);
        })
        .catch((error) => {
          if (cancelled) return;
          setResults([]);
          onError(errorMessage(error));
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, onError, hasApiKey]);

  // Local shelf entries store only a clip id, so resolve a current thumbnail.
  useEffect(() => {
    const ids = [
      ...new Set(
        favorites.flatMap((favorite) =>
          favorite.clipId === null ? [] : [favorite.clipId],
        ),
      ),
    ];
    if (ids.length === 0) {
      setClipThumbs({});
      return;
    }
    let cancelled = false;
    void Promise.all(
      ids.map(async (id) => {
        try {
          const detail = await getClip(id);
          // Animated clips keep a still thumbnail beside the recovered animation,
          // so hover-to-play can show the still first.
          return [
            id,
            {
              still: detail.thumbFile,
              animated: detail.animated ? `${id}.gif` : detail.thumbFile,
            },
          ] as const;
        } catch {
          return [id, { still: null, animated: null }] as const;
        }
      }),
    ).then((entries) => {
      if (!cancelled) setClipThumbs(Object.fromEntries(entries));
    });
    return () => {
      cancelled = true;
    };
  }, [favorites]);

  // Mirror the grid slider: follow the drag locally, persist once it settles.
  useEffect(() => {
    if (liveScale === null) return;
    const timer = window.setTimeout(() => onScaleCommit(liveScale / 100), 300);
    return () => window.clearTimeout(timer);
  }, [liveScale, onScaleCommit]);
  useEffect(() => {
    if (liveScale !== null && Math.round(scale * 100) === liveScale) setLiveScale(null);
  }, [scale, liveScale]);

  const favoriteIds = useMemo(() => new Set(favorites.map((favorite) => favorite.id)), [favorites]);

  const toggleResult = (result: GifResult) => {
    const favorite = gifFavoriteFromResult(result);
    const next = favoriteIds.has(favorite.id)
      ? favorites.filter((entry) => entry.id !== favorite.id)
      : [favorite, ...favorites];
    onFavoritesChange(next.slice(0, 100));
  };

  const removeFavorite = async (favorite: GifFavorite) => {
    // A clip-backed entry keeps its media in the hidden archive; releasing the
    // clip there is what lets it return to the history.
    if (favorite.clipId !== null) {
      try {
        await setClipSavedGif(favorite.clipId, false);
      } catch (error) {
        onError(errorMessage(error));
        return;
      }
    }
    onFavoritesChange(favorites.filter((entry) => entry.id !== favorite.id));
  };

  const copyResult = async (result: GifResult) => {
    try {
      await copyGif(result.url);
      onNotice(`Copied “${result.title || "GIF"}”. Paste it into a chat or editor.`);
    } catch (error) {
      onError(errorMessage(error));
    }
  };

  const copyLocal = async (favorite: GifFavorite) => {
    try {
      await copyCustomGif(favorite.previewUrl);
      onNotice(`Copied “${favorite.title || "GIF"}”. Paste it into a chat or editor.`);
    } catch (error) {
      onError(errorMessage(error));
    }
  };

  const addCustomGif = async () => {
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "GIF images", extensions: ["gif"] }],
      });
      if (typeof chosen !== "string") return;
      const favorite = await importCustomGif(chosen);
      onFavoritesChange(
        [favorite, ...favorites.filter((entry) => entry.id !== favorite.id)].slice(0, 100),
      );
      onNotice(`Added “${favorite.title || "GIF"}” to your GIFs.`);
    } catch (error) {
      onError(errorMessage(error));
    }
  };

  const shelf = favorites.filter((favorite) => {
    if (favorite.clipId !== null) return clipThumbs[favorite.clipId] != null;
    return favorite.previewUrl.length > 0;
  });

  // The same query narrows the saved shelf, not just the GIPHY results.
  const trimmedQuery = query.trim();
  const normalizedQuery = trimmedQuery.toLowerCase();
  const savedGifs = normalizedQuery
    ? shelf.filter((favorite) => favorite.title.toLowerCase().includes(normalizedQuery))
    : shelf;

  /** Still and animation for a shelf entry, when both are known. */
  const shelfSources = (favorite: GifFavorite) => {
    if (favorite.clipId !== null) {
      const thumb = clipThumbs[favorite.clipId];
      return {
        still: thumb?.still ? mediaUrl(thumb.still) : null,
        animated: thumb?.animated ? mediaUrl(thumb.animated) : null,
      };
    }
    if (favorite.id.startsWith("local:")) {
      // Imported files have no still frame beside them, so they keep playing.
      return { still: null, animated: mediaUrl(favorite.previewUrl) };
    }
    const preview = favorite.previewUrl || favorite.url;
    return {
      still: favorite.stillUrl || preview,
      animated: preview,
    };
  };

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
          placeholder="Search GIPHY…  (happy dance, facepalm, hello)"
          aria-label="Search GIPHY"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-faint focus:outline-none"
        />
        <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">
          {loading ? "Searching…" : `${results.length} GIF${results.length === 1 ? "" : "s"}`}
        </span>
        {query.length > 0 && (
          <button
            type="button"
            onClick={() => setQuery("")}
            title="Clear the GIF search"
            aria-label="Clear the GIF search"
            className="shrink-0 rounded p-0.5 text-ink-faint transition-colors hover:text-ink"
          >
            <IconClose className="size-3" />
          </button>
        )}
      </div>

      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-edge bg-panel px-3 py-1.5">
        <span className="text-[10px] text-ink-faint">Preview size</span>
        <input
          type="range"
          min={60}
          max={200}
          step={5}
          value={percent}
          onChange={(event) => setLiveScale(Number(event.target.value))}
          aria-label="GIF preview size"
          className="h-1 w-28 cursor-pointer accent-[var(--color-accent)]"
        />
        <span className="w-8 text-[10px] tabular-nums text-ink-faint">{percent}%</span>
        <button
          type="button"
          onClick={() => onLoopChange(!loop)}
          aria-pressed={loop}
          title={
            loop
              ? "GIFs loop continuously — turn off to play only while hovered"
              : "GIFs play only while hovered — turn on to loop continuously"
          }
          className={`flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] transition-colors ${
            loop
              ? "border-accent bg-accent/20 text-accent-hi"
              : "border-edge bg-panel-2 text-ink-faint hover:text-ink-dim"
          }`}
        >
          <IconLoop className="size-3" />
          Loop
        </button>
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => void addCustomGif()}
          title="Add a GIF file from this computer to My GIFs"
          className="flex shrink-0 items-center gap-1 rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
        >
          <IconPlus className="size-3" />
          Add GIF file
        </button>
        <span className="flex items-center gap-1 text-[9px] uppercase tracking-wider text-ink-faint">
          <IconGif className="size-3" />
          Powered by GIPHY
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
        {!hasApiKey && (
          <div className="mb-3 rounded border border-edge bg-panel-2 px-3 py-2 text-[11px] leading-relaxed text-ink-dim">
            Add a free GIPHY API key in{" "}
            <span className="text-ink">Settings → Utilities</span> to search. Saved GIFs and
            clips on the shelf below keep working without one.
          </div>
        )}

        {shelf.length > 0 && (
          <section className="pb-3">
            <h3 className="pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
              Your GIFs
            </h3>
            {savedGifs.length === 0 ? (
              <p className="pb-1 text-[10px] leading-relaxed text-ink-faint">
                No saved GIFs match “{trimmedQuery}”.
              </p>
            ) : (
              <div className="flex flex-wrap gap-2 pb-1">
                {savedGifs.map((favorite) => {
                  const sources = shelfSources(favorite);
                  return (
                  <div key={favorite.id} className="group relative shrink-0">
                    {favorite.clipId !== null ? (
                      <GifCard
                        title={favorite.title}
                        still={sources.still}
                        animated={sources.animated}
                        loop={loop}
                        width={Math.round(minWidth * 0.8)}
                        height={Math.round(mediaHeight * 0.8)}
                        favorite={null}
                        onCopy={() => onCopyClip(favorite.clipId as number)}
                      />
                    ) : favorite.id.startsWith("local:") ? (
                      <GifCard
                        title={favorite.title}
                        still={sources.still}
                        animated={sources.animated}
                        loop={loop}
                        width={Math.round(minWidth * 0.8)}
                        height={Math.round(mediaHeight * 0.8)}
                        favorite={null}
                        onCopy={() => void copyLocal(favorite)}
                      />
                    ) : (
                      <GifCard
                        title={favorite.title}
                        still={sources.still}
                        animated={sources.animated}
                        loop={loop}
                        width={Math.round(minWidth * 0.8)}
                        height={Math.round(mediaHeight * 0.8)}
                        favorite={null}
                        onCopy={() =>
                          void copyResult({
                            id: favorite.id,
                            title: favorite.title,
                            url: favorite.url,
                            previewUrl: favorite.previewUrl,
                            stillUrl: favorite.stillUrl,
                            width: 0,
                            height: 0,
                          })
                        }
                      />
                    )}
                    <button
                      type="button"
                      onClick={() => void removeFavorite(favorite)}
                      title="Remove from My GIFs"
                      aria-label="Remove from My GIFs"
                      className="absolute -right-1 -top-1 hidden rounded-full border border-edge bg-panel-3 p-0.5 text-ink-faint hover:text-danger group-hover:block"
                    >
                      <IconClose className="size-2.5" />
                    </button>
                  </div>
                  );
                })}
              </div>
            )}
            <p className="pt-1 text-[10px] leading-relaxed text-ink-faint">
              Starred GIFs, image clips from the preview pane and GIF files you added live
              here.
            </p>
          </section>
        )}

        <section>
          <h3 className="pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
            {query.trim() ? `Results for “${query.trim()}”` : "Trending on GIPHY"}
          </h3>
          {!hasApiKey ? (
            <p className="py-8 text-center text-[11px] text-ink-faint">
              GIPHY search is off until a key is added.
            </p>
          ) : loading && results.length === 0 ? (
            <p className="py-8 text-center text-[11px] text-ink-faint">Searching GIPHY…</p>
          ) : results.length === 0 ? (
            <p className="py-8 text-center text-[11px] text-ink-faint">
              {query.trim()
                ? `No GIFs match “${query.trim()}”.`
                : "No trending GIFs right now. Try a search."}
            </p>
          ) : (
            <div
              className="grid gap-2"
              style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${minWidth}px, 1fr))` }}
            >
              {results.map((result) => (
                <GifCard
                  key={result.id}
                  title={result.title}
                  still={result.stillUrl || result.previewUrl}
                  animated={result.previewUrl}
                  loop={loop}
                  width={minWidth}
                  height={mediaHeight}
                  favorite={favoriteIds.has(`giphy:${result.id}`)}
                  onCopy={() => void copyResult(result)}
                  onToggleFavorite={() => toggleResult(result)}
                />
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
