import { useEffect } from "react";
import { IconExternal } from "./icons";

interface WhatsNewDialogProps {
  /** The version that was just installed. */
  version: string;
  /** The version that was running before the update. */
  previous: string | null;
  onOpenChangelog: () => void;
  onDismiss: () => void;
}

/** Shown once after an update, so the release notes are one click away. */
export function WhatsNewDialog({
  version,
  previous,
  onOpenChangelog,
  onDismiss,
}: WhatsNewDialogProps) {
  // Escape belongs to the dialog rather than to the handler that hides the window.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onDismiss();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onDismiss]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 p-6 animate-fade-in"
      onMouseDown={onDismiss}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="What's new"
        onMouseDown={(event) => event.stopPropagation()}
        className="w-full max-w-[320px] rounded-panel border border-edge bg-panel p-4 shadow-xl animate-rise"
      >
        <h2 className="text-[13px] font-semibold text-ink">
          Updated to CopyPaste {version}
        </h2>
        <p className="mt-1.5 text-[11px] leading-relaxed text-ink-dim">
          {previous
            ? `You were on ${previous}. Every change in this release is listed in the GitHub notes.`
            : "Every change in this release is listed in the GitHub notes."}
        </p>

        <div className="mt-3 flex items-center justify-end gap-1.5">
          <button
            type="button"
            onClick={onDismiss}
            className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
          >
            Not now
          </button>
          <button
            type="button"
            autoFocus
            onClick={onOpenChangelog}
            className="flex items-center gap-1 rounded border border-accent-dim bg-accent/20 px-2.5 py-1 text-[11px] text-ink transition-colors hover:bg-accent/30"
          >
            <IconExternal className="size-3" />
            What&apos;s new
          </button>
        </div>
      </div>
    </div>
  );
}
