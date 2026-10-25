import { useState, type ReactNode } from "react";
import { useVaultGate } from "../lib/vaultGate";
import { IconShield } from "./icons";

interface SecretPreviewProps {
  /** Blur the content until the user asks to see it. */
  level: string | null;
  children: ReactNode;
  /**
   * The clip's plain text, when the caller has it. A concealed secret shows its
   * own ends so two credentials can be told apart without either being readable.
   */
  text?: string;
  /** Extra classes for the wrapper. */
  className?: string;
  /**
   * Pins the reveal control to the top-left corner of the closest positioned
   * ancestor instead of ending the text line, so it never sits under the card's
   * hover actions.
   */
  corner?: boolean;
}

/**
 * How much of a secret stays readable: the first character, and the last three —
 * or only the last one when the value is too short for that to give anything
 * away. Everything between the two is the part that stays hidden.
 *
 * Returned as three pieces rather than one string because only the middle is
 * blurred; masking the ends would defeat the purpose and revealing the middle
 * would defeat the secret.
 */
export function maskSecret(text: string): { head: string; middle: string; tail: string } {
  const chars = [...text];
  const total = chars.length;
  if (total === 0) return { head: "", middle: "", tail: "" };
  if (total === 1) return { head: chars[0], middle: "", tail: "" };

  // Under eight characters the last three would be most of the value, so a short
  // secret gives up only its final character.
  const tailLength = total >= 8 ? 3 : 1;
  return {
    head: chars[0],
    middle: chars.slice(1, total - tailLength).join(""),
    tail: chars.slice(total - tailLength).join(""),
  };
}

/** The concealed form of a secret: readable ends, blurred middle. */
export function MaskedSecret({ text }: { text: string }) {
  const { head, middle, tail } = maskSecret(text);
  return (
    <span title="Hidden until you reveal it">
      {head}
      {middle && <span className="blur-[3px] select-none">{middle}</span>}
      {tail}
    </span>
  );
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
export function SecretPreview({
  level,
  children,
  text,
  className,
  corner = false,
}: SecretPreviewProps) {
  const [revealed, setRevealed] = useState(false);
  const vault = useVaultGate();
  if (!level) return <>{children}</>;

  // With the text in hand the concealment can be a mask rather than a blur: the
  // ends stay legible and everything between them is drawn blurred, which reads
  // as "there is a value here" instead of "something is hidden".
  // A sealed clip read while the vault is locked has no text at all. Showing an
  // empty row would look like a bug, so it says what it is.
  const locked = text !== undefined && text.length === 0;
  const concealed =
    text !== undefined && !revealed && !locked ? <MaskedSecret text={text} /> : children;

  if (corner) {
    return (
      <span className={`block min-w-0 ${className ?? ""}`}>
        <span className={`block min-w-0 truncate ${revealed ? "" : "select-none"}`}>
          {concealed}
        </span>
        {/* The corner control is a toggle for the same reason as the row's. */}
        <button
          type="button"
          title={
            locked
              ? "Unlock the vault to read this clip"
              : revealed
                ? "Hide this clip again"
                : secretLabel(level)
          }
          aria-label={
            locked
              ? "Locked: unlock the vault to read this clip"
              : revealed
                ? "Hide this clip again"
                : `Reveal this clip: ${secretLabel(level).toLowerCase()}`
          }
          aria-pressed={revealed}
          onClick={(event) => {
            event.stopPropagation();
            if (locked) vault.requestUnlock();
            else setRevealed(!revealed);
          }}
          className={`absolute left-1 top-1 z-10 flex items-center rounded bg-canvas/85 p-0.5 text-warn transition-colors hover:bg-warn/25 ${
            revealed ? "opacity-70 hover:opacity-100" : ""
          }`}
        >
          <IconShield className="size-3" />
        </button>
      </span>
    );
  }

  return (
    <span className={`relative flex min-w-0 items-center gap-1.5 ${className ?? ""}`}>
      <span className={`min-w-0 truncate ${revealed ? "" : "select-none"}`}>
        {concealed}
      </span>
      {/* The control stays put once revealed: disappearing left no way to hide the
          clip again short of selecting something else. */}
      <button
        type="button"
        title={
          locked
            ? "Unlock the vault to read this clip"
            : revealed
              ? "Hide this clip again"
              : secretLabel(level)
        }
        aria-label={
          locked
            ? "Locked: unlock the vault to read this clip"
            : revealed
              ? "Hide this clip again"
              : `Reveal this clip: ${secretLabel(level).toLowerCase()}`
        }
        aria-pressed={revealed}
        onClick={(event) => {
          event.stopPropagation();
          // Nothing to reveal while locked, so the press asks for the key that
          // would make it possible instead of doing nothing.
          if (locked) vault.requestUnlock();
          else setRevealed(!revealed);
        }}
        className="flex shrink-0 items-center gap-0.5 rounded bg-warn/20 px-1 py-px text-[9px] text-warn transition-colors hover:bg-warn/30"
      >
        <IconShield className="size-2.5" />
        {locked ? "Locked" : revealed ? "Hide" : "Reveal"}
      </button>
    </span>
  );
}
