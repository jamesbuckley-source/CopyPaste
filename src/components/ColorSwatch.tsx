/**
 * Swatch for a recognised color clip.
 *
 * The code is painted *on* the color, so its own color is chosen from the
 * swatch's luminance — a fixed color would vanish on light or dark samples.
 */

/** `#ffffff` or `#101010`, whichever stays readable on `hex`. */
export function readableOn(hex: string): string {
  const value = hex.replace("#", "");
  const r = Number.parseInt(value.slice(0, 2), 16) || 0;
  const g = Number.parseInt(value.slice(2, 4), 16) || 0;
  const b = Number.parseInt(value.slice(4, 6), 16) || 0;
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.6 ? "#101010" : "#ffffff";
}

/** `rgb(r, g, b)` for display next to the stored hex value. */
export function rgbLabel(hex: string): string {
  const value = hex.replace("#", "");
  const r = Number.parseInt(value.slice(0, 2), 16) || 0;
  const g = Number.parseInt(value.slice(2, 4), 16) || 0;
  const b = Number.parseInt(value.slice(4, 6), 16) || 0;
  return `rgb(${r}, ${g}, ${b})`;
}

export function ColorSwatch({
  color,
  label,
  className = "",
  textClassName = "text-[10px]",
}: {
  color: string;
  /** Usually the copied code; omitted for thumbnails. */
  label?: string;
  className?: string;
  textClassName?: string;
}) {
  const ink = readableOn(color);
  return (
    <div
      title={label ? `${color} · ${label}` : color}
      className={`relative flex shrink-0 items-center justify-center overflow-hidden rounded border border-edge ${className}`}
      style={{ backgroundColor: color }}
    >
      {label && (
        <span
          className={`max-w-full truncate rounded px-1.5 py-0.5 font-mono ${textClassName}`}
          style={{
            color: ink,
            backgroundColor: ink === "#ffffff" ? "rgba(0, 0, 0, 0.38)" : "rgba(255, 255, 255, 0.6)",
          }}
        >
          {label}
        </span>
      )}
    </div>
  );
}
