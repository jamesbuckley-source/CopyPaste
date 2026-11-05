import { useEffect, useRef, useState } from "react";

interface AnimatedImageProps {
  /** Static frame shown while the animation is not playing. */
  still: string | null;
  /** Animated source, or null when the image does not move. */
  animated: string | null;
  /** Video source, or null when the moving asset is a GIF. */
  video?: string | null;
  /** True keeps the animation running without hover; false plays it on hover. */
  loop?: boolean;
  alt?: string;
  className?: string;
  loading?: "lazy" | "eager";
}

/**
 * Draws an animation into a canvas so it can be held still.
 *
 * Used when there is no separate static frame (imported GIF files). The first
 * frame is painted once and repainted every animation frame only while the
 * pointer is over the canvas.
 */
function CanvasAnimation({
  src,
  loop,
  className,
  onError,
}: {
  src: string;
  loop: boolean;
  className?: string;
  onError: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const frameRef = useRef(0);

  const paint = () => {
    const canvas = canvasRef.current;
    const image = imageRef.current;
    if (!canvas || !image || image.naturalWidth === 0) return;
    if (canvas.width !== image.naturalWidth) canvas.width = image.naturalWidth;
    if (canvas.height !== image.naturalHeight) canvas.height = image.naturalHeight;
    canvas.getContext("2d")?.drawImage(image, 0, 0);
  };

  useEffect(() => {
    const image = new Image();
    image.decoding = "async";
    image.onload = paint;
    // A file the window cannot read — a cloud folder that is busy, or a GIF that
    // was deleted — has to say so rather than leave an empty card.
    image.onerror = onError;
    image.src = src;
    imageRef.current = image;
    if (image.complete) paint();
    return () => {
      image.onload = null;
      image.onerror = null;
      cancelAnimationFrame(frameRef.current);
      frameRef.current = 0;
    };
    // `paint` only reads refs, so the source is the only dependency that matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src]);

  useEffect(() => {
    if (!loop) return;
    let active = true;
    const tick = () => {
      paint();
      if (active) frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
    return () => {
      active = false;
      cancelAnimationFrame(frameRef.current);
      frameRef.current = 0;
    };
  }, [loop]);

  const start = () => {
    if (loop) return;
    cancelAnimationFrame(frameRef.current);
    const tick = () => {
      paint();
      frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
  };

  const stop = () => {
    if (loop) return;
    cancelAnimationFrame(frameRef.current);
    frameRef.current = 0;
  };

  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      onMouseEnter={start}
      onMouseLeave={stop}
      className={className}
    />
  );
}

/**
 * A `<video>` that behaves like the rest of the media grid.
 *
 * Always muted: the app is a clipboard, not a player, and a page's clip should
 * never start talking. `loop` mirrors the GIF setting, so "always running" and
 * "only while hovered" work the same for both kinds of moving asset. The still
 * frame is the poster, so nothing decodes until the video is asked to play.
 */
function VideoMotion({
  src,
  poster,
  loop,
  alt,
  className,
  onError,
}: {
  src: string;
  poster: string | null;
  loop: boolean;
  alt: string;
  className?: string;
  onError: () => void;
}) {
  const ref = useRef<HTMLVideoElement | null>(null);

  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    if (loop) {
      void video.play().catch(() => undefined);
    } else {
      video.pause();
      video.currentTime = 0;
    }
  }, [loop]);

  const play = () => {
    if (loop) return;
    void ref.current?.play().catch(() => undefined);
  };

  const pause = () => {
    const video = ref.current;
    if (loop || !video) return;
    video.pause();
    video.currentTime = 0;
  };

  return (
    <video
      ref={ref}
      src={src}
      poster={poster ?? undefined}
      muted
      loop
      playsInline
      // Nothing is fetched until the pointer asks for it — unless there is no
      // poster, where a first frame has to load for the card to show anything.
      preload={poster ? "none" : "metadata"}
      aria-label={alt}
      onError={onError}
      onMouseEnter={play}
      onMouseLeave={pause}
      className={className}
    />
  );
}

/**
 * An `<img>` that can hold an animation still, or a muted video player.
 *
 * A GIF cannot be paused from CSS, so "loop only on hover" is implemented by
 * swapping the source: the still frame at rest, the animation while the pointer
 * is over the image. Without a still frame the animation is drawn to a canvas
 * that only advances while hovered. A video needs no swapping: the element is
 * paused at its poster and played on hover.
 */
export function AnimatedImage({
  still,
  animated,
  video = null,
  loop = false,
  alt = "",
  className,
  loading = "lazy",
}: AnimatedImageProps) {
  // Declared before the video branch so the hook order never depends on props.
  const [hovered, setHovered] = useState(false);
  /** Set when a source failed to load; cleared by a retry or a new source. */
  const [failed, setFailed] = useState(false);
  /** Bumped by the retry, so the browser asks for the file again. */
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    setFailed(false);
  }, [still, animated, video, attempt]);

  if (failed) {
    return (
      <button
        type="button"
        onClick={() => setAttempt((count) => count + 1)}
        title="This file could not be read — click to try again"
        className={`flex flex-col items-center justify-center gap-1 border border-edge-soft bg-panel-2 text-[10px] text-ink-faint transition-colors hover:text-ink ${className ?? ""}`}
      >
        <span>Couldn’t load</span>
        <span className="underline">Retry</span>
      </button>
    );
  }

  // A retry has to look like a different address, or the webview serves the cached
  // failure straight back.
  const retried = (source: string | null) =>
    source === null
      ? null
      : attempt === 0
        ? source
        : `${source}${source.includes("?") ? "&" : "?"}retry=${attempt}`;

  if (video) {
    return (
      <VideoMotion
        src={retried(video) ?? video}
        poster={retried(still)}
        loop={loop}
        alt={alt}
        className={className}
        onError={() => setFailed(true)}
      />
    );
  }

  const source = retried(animated ?? still);
  if (!source) return null;

  if (animated && (!still || still === animated)) {
    // No separate static frame: draw the animation to a canvas so it can be held
    // on its current frame. This is what makes older favourites and imported GIF
    // files pause like the GIPHY results do.
    return (
      <CanvasAnimation
        src={source}
        loop={loop}
        className={className}
        onError={() => setFailed(true)}
      />
    );
  }

  if (animated) {
    const playing = loop || hovered;
    return (
      <img
        src={playing ? source : (retried(still) ?? source)}
        alt={alt}
        loading={loading}
        draggable={false}
        onError={() => setFailed(true)}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        className={className}
      />
    );
  }

  return (
    <img
      src={source}
      alt={alt}
      loading={loading}
      draggable={false}
      onError={() => setFailed(true)}
      className={className}
    />
  );
}
