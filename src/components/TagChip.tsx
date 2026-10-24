import type { ReactNode } from "react";
import { tagChipColors } from "../lib/tagColor";

/**
 * One tag chip, shared by the list rows, the card grid and the properties pane.
 *
 * A tag with a colour gets a tinted surface and luminance-matched ink; a tag
 * without one keeps the neutral accent chip. `className` carries each call
 * site's size, padding and hover, so the chip occupies exactly the space it
 * did before.
 */
export function TagChip({
  name,
  color,
  className = "",
  title,
  onClick,
  children,
}: {
  name: string;
  color?: string | null;
  className?: string;
  title?: string;
  /** Renders the chip as a button; the properties pane uses this to remove. */
  onClick?: () => void;
  children?: ReactNode;
}) {
  const colors = tagChipColors(color);
  const style = colors
    ? {
        backgroundColor: colors.background,
        color: colors.text,
        // An inset ring rather than a border, so the chip does not grow by a pixel.
        boxShadow: `inset 0 0 0 1px ${colors.border}`,
      }
    : undefined;
  const classes = `rounded bg-accent/10 text-accent-hi ${className}`;
  const content = (
    <>
      #{name}
      {children}
    </>
  );

  if (onClick) {
    return (
      <button type="button" title={title} onClick={onClick} className={classes} style={style}>
        {content}
      </button>
    );
  }

  return (
    <span title={title} className={classes} style={style}>
      {content}
    </span>
  );
}
