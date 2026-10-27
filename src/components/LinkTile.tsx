import { linkHost, linkHue, linkInitial } from "../lib/linkPreview";

/**
 * A link clip's placeholder picture: the site's initial on a tile tinted from its
 * host, with the host itself underneath when there is room.
 *
 * Drawn from the address alone — no network, nothing stored — so a copied link is
 * recognisable at a glance before (or instead of) fetching the page's own image.
 */
export function LinkTile({
  url,
  className,
  letterClassName,
  showHost = false,
}: {
  url: string;
  className?: string;
  letterClassName?: string;
  /** True in the card layout, where there is room for the host under the letter. */
  showHost?: boolean;
}) {
  const host = linkHost(url);
  const hue = linkHue(host);
  return (
    <span
      aria-hidden="true"
      className={`flex flex-col items-center justify-center gap-0.5 overflow-hidden px-1 ${className ?? ""}`}
      style={{
        backgroundColor: `hsl(${hue} 45% 45% / 0.18)`,
        boxShadow: `inset 0 0 0 1px hsl(${hue} 45% 55% / 0.3)`,
      }}
    >
      <span
        className={`font-semibold leading-none ${letterClassName ?? "text-[14px]"}`}
        style={{ color: `hsl(${hue} 70% 62%)` }}
      >
        {linkInitial(host)}
      </span>
      {showHost && (
        <span className="max-w-full truncate text-[8px] uppercase tracking-wide text-ink-faint">
          {host}
        </span>
      )}
    </span>
  );
}
