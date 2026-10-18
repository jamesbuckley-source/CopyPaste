import { useEffect, useState } from "react";

interface PromptDialogProps {
  title: string;
  description?: string;
  label?: string;
  initialValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  /** Renders the field as a colour picker instead of a text box. */
  kind?: "text" | "color";
  /** Extra one-click answers, e.g. "Remove colour". */
  choices?: { label: string; value: string }[];
  onCancel: () => void;
  onSubmit: (value: string) => void;
}

/**
 * Small modal for the one-line answers the sidebar and bulk bar need (rename a
 * tag, pick a colour, name a saved search).
 *
 * The frameless window has no native prompt, and `window.prompt` is unavailable in
 * a webview, so this stands in for both.
 */
export function PromptDialog({
  title,
  description,
  label,
  initialValue = "",
  placeholder,
  confirmLabel = "Save",
  kind = "text",
  choices,
  onCancel,
  onSubmit,
}: PromptDialogProps) {
  const [value, setValue] = useState(initialValue);

  // Escape belongs to the dialog, not to the window handler that hides the app.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onCancel]);

  const submit = () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 p-6 animate-fade-in"
      onMouseDown={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
        className="w-full max-w-[320px] rounded-panel border border-edge bg-panel p-4 shadow-xl animate-rise"
      >
        <h2 className="text-[13px] font-semibold text-ink">{title}</h2>
        {description && (
          <p className="mt-1.5 text-[11px] leading-relaxed text-ink-dim">{description}</p>
        )}

        <form
          className="mt-2.5 space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          {label && (
            <label className="block text-[10px] font-medium uppercase tracking-wider text-ink-faint">
              {label}
            </label>
          )}
          <input
            autoFocus
            type={kind === "color" ? "color" : "text"}
            value={value || (kind === "color" ? "#7c9cff" : "")}
            onChange={(event) => setValue(event.target.value)}
            onKeyDown={(event) => event.stopPropagation()}
            placeholder={placeholder}
            spellCheck={false}
            aria-label={label ?? title}
            className={`w-full rounded border border-edge bg-panel-2 px-2 py-1 text-[12px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none ${
              kind === "color" ? "h-8" : ""
            }`}
          />

          {choices && choices.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {choices.map((choice) => (
                <button
                  key={choice.value}
                  type="button"
                  onClick={() => onSubmit(choice.value)}
                  className="rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                >
                  {choice.label}
                </button>
              ))}
            </div>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onCancel}
              className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="rounded border border-accent bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-accent/90"
            >
              {confirmLabel}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
