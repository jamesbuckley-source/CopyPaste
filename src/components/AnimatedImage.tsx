import { useState } from "react";

interface AnimatedImageProps {
  /** Static frame shown while the animation is not playing. */
  still: string | null;
  /** Animated source, or null when the image does not move. */
  animated: string | null;
  /** True keeps the animation running without hover; false plays it on hover. */
  loop?: boolean;
  alt?: string;
  className?: string;
  loading?: "lazy" | "eager";
}

/**
 * An `<img>` that can hold an animation still.
 *
 * A GIF cannot be paused from CSS, so "loop only on hover" is implemented by
 * swapping the source: the still frame at rest, the animation while the pointer
 * is over the image. Without a still frame the animation simply keeps playing.
 */
export function AnimatedImage({
  still,
  animated,
  loop = false,
  alt = "",
  className,
  loading = "lazy",
}: AnimatedImageProps) {
  const [hovered, setHovered] = useState(false);
  const source = animated ?? still;
  if (!source) return null;

  const playing = animated !== null && (loop || hovered) && still !== animated;

  return (
    <img
      src={playing || !still ? source : still}
      alt={alt}
      loading={loading}
      draggable={false}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className={className}
    />
  );
}
