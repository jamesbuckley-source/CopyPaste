import type { RefObject } from "react";

import { MARKDOWN_BUTTONS, applyFormat, type Format } from "../lib/markdownFormat";

interface MarkdownToolbarProps {
  /** Textarea whose value and selection each button formats. */
  areaRef: RefObject<HTMLTextAreaElement | null>;
  /** Receives the edited value; whatever the caller stores drives the save. */
  onChange: (value: string) => void;
  /** Chrome for the surrounding bar; the button row itself is shared. */
  className?: string;
}

/**
 * The Markdown formatting bar used wherever clip text is edited.
 *
 * The buttons never take focus away from the textarea (`mousedown` is
 * suppressed), so the selection the button acts on is still the user's, and it
 * is put back after the edited value has been rendered.
 */
export function MarkdownToolbar({ areaRef, onChange, className }: MarkdownToolbarProps) {
  const apply = (format: Format) => {
    const area = areaRef.current;
    if (!area) return;
    const next = applyFormat(area.value, area.selectionStart, area.selectionEnd, format);
    onChange(next.value);
    // The value reaches the DOM on the next paint, so the selection is restored
    // after that; otherwise the caret would jump to the end.
    requestAnimationFrame(() => {
      area.focus();
      area.setSelectionRange(next.start, next.end);
    });
  };

  return (
    <div className={`flex flex-wrap items-center gap-1 ${className ?? ""}`}>
      {MARKDOWN_BUTTONS.map((button) => (
        <button
          key={button.label}
          type="button"
          // Keeping focus in the textarea preserves the selection being wrapped.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => apply(button.format)}
          title={button.title}
          aria-label={button.title}
          className={`rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink ${button.className ?? ""}`}
        >
          {button.label}
        </button>
      ))}
    </div>
  );
}
