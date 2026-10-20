import { useState, type ReactNode } from "react";
import { IconShield } from "./icons";

interface SecretPreviewProps {
  /** Blur the content until the user asks to see it. */
  level: string | null;
  children: ReactNode;
  /** Extra classes for the wrapper. */
  className?: string;
  /**
   * Pins the reveal control to the top-left corner of the closest positioned
   * ancestor instead of ending the text line, so it never sits under the card's
   * hover actions.
   */
  corner?: boolean;
}

/** Near-verbatim label for a detection confidence. */
export function secretLabel(level: string | null): string {
  if (level === "high") return "Likely secret";
  if (level === "medium") return "Possible secret";
  return "Maybe a secret";
}

/**
 * Blurs a preview that looks like a credential. The text stays selectable once
 * revealed, and revealing never changes what the row does when clicked.
 */
export function SecretPreview({ level, children, className, corner = false }: SecretPreviewProps) {
  const [revealed, setRevealed] = useState(false);
  if (!level) return <>{children}</>;

  if (corner) {
    return (
      <span className={`block min-w-0 ${className ?? ""}`}>
        <span className={`block min-w-0 truncate ${revealed ? "" : "blur-[3px] select-none"}`}>
          {children}
        </span>
        {!revealed && (
          <button
            type="button"
            title={secretLabel(level)}
            aria-label={`Reveal this clip: ${secretLabel(level).toLowerCase()}`}
            onClick={(event) => {
              event.stopPropagation();
              setRevealed(true);
            }}
            className="absolute left-1 top-1 z-10 flex items-center rounded bg-canvas/85 p-0.5 text-warn transition-colors hover:bg-warn/25"
          >
            <IconShield className="size-3" />
          </button>
        )}
      </span>
    );
  }

  return (
    <span className={`relative flex min-w-0 items-center gap-1.5 ${className ?? ""}`}>
      <span className={`min-w-0 truncate ${revealed ? "" : "blur-[3px] select-none"}`}>
        {children}
      </span>
      {!revealed && (
        <button
          type="button"
          title={secretLabel(level)}
          aria-label={`Reveal this clip: ${secretLabel(level).toLowerCase()}`}
          onClick={(event) => {
            event.stopPropagation();
            setRevealed(true);
          }}
          className="flex shrink-0 items-center gap-0.5 rounded bg-warn/20 px-1 py-px text-[9px] text-warn transition-colors hover:bg-warn/30"
        >
          <IconShield className="size-2.5" />
          Reveal
        </button>
      )}
    </span>
  );
}
