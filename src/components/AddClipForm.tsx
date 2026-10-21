import { useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createTextClip, errorMessage } from "../lib/ipc";

/** How a toolbar button edits the text around the current selection. */
type Format =
  /** Wraps the selection, or places the caret between the markers. */
  | { kind: "wrap"; marker: string }
  /** Prefixes the touched lines, which is how headings and lists are made. */
  | { kind: "prefix"; marker: string }
  | { kind: "link" };

interface ToolbarButton {
  label: string;
  title: string;
  format: Format;
  className?: string;
}

const BUTTONS: ToolbarButton[] = [
  { label: "B", title: "Bold  **text**", format: { kind: "wrap", marker: "**" }, className: "font-bold" },
  { label: "I", title: "Italic  *text*", format: { kind: "wrap", marker: "*" }, className: "italic" },
  { label: "</>", title: "Inline code  `code`", format: { kind: "wrap", marker: "`" } },
  { label: "H1", title: "Heading  # text", format: { kind: "prefix", marker: "# " } },
  { label: "• List", title: "Bullet list  - text", format: { kind: "prefix", marker: "- " } },
  { label: "Link", title: "Link  [text](url)", format: { kind: "link" } },
];

interface TextEdit {
  value: string;
  start: number;
  end: number;
}

/**
 * Applies one formatting action to `text` and reports the caret/selection that
 * should follow, so the caller can restore it after the value updates.
 */
function applyFormat(text: string, start: number, end: number, format: Format): TextEdit {
  const selected = text.slice(start, end);

  if (format.kind === "wrap") {
    const { marker } = format;
    return {
      value: `${text.slice(0, start)}${marker}${selected}${marker}${text.slice(end)}`,
      start: start + marker.length,
      end: end + marker.length,
    };
  }

  if (format.kind === "link") {
    const label = selected || "text";
    const urlStart = start + 1 + label.length + 2;
    return {
      value: `${text.slice(0, start)}[${label}](url)${text.slice(end)}`,
      // Selecting the placeholder means the next keystroke replaces it.
      start: urlStart,
      end: urlStart + 3,
    };
  }

  // A prefix applies to every line the selection touches, or to the caret's own.
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const lineEnd = text.indexOf("\n", end);
  const blockEnd = lineEnd === -1 ? text.length : lineEnd;
  const lines = text.slice(lineStart, blockEnd).split("\n");
  const prefixed = lines.map((line) => `${format.marker}${line}`).join("\n");
  // Every marker inserted before a position pushes that position along, which is
  // what keeps the caret (or selection) on the same text after the edit.
  const lineStarts: number[] = [];
  let cursor = lineStart;
  for (const line of lines) {
    lineStarts.push(cursor);
    cursor += line.length + 1;
  }
  const shift = (position: number) =>
    lineStarts.filter((linePosition) => linePosition <= position).length *
    format.marker.length;
  return {
    value: `${text.slice(0, lineStart)}${prefixed}${text.slice(blockEnd)}`,
    start: start + shift(start),
    end: end + shift(end),
  };
}

/**
 * The body of the standalone "New clip" window.
 *
 * It is deliberately small: type (optionally with the Markdown toolbar), save, and
 * the window closes. Whether the clip ends up badged as Markdown is decided in the
 * backend from the saved text, so this form never has to agree with it.
 */
export function AddClipForm() {
  const areaRef = useRef<HTMLTextAreaElement | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const close = () => {
    getCurrentWindow()
      .close()
      .catch((e: unknown) => setError(errorMessage(e)));
  };

  const apply = (format: Format) => {
    const area = areaRef.current;
    if (!area) return;
    const next = applyFormat(area.value, area.selectionStart, area.selectionEnd, format);
    setText(next.value);
    // The value reaches the DOM on the next paint, so the selection is restored
    // after that; otherwise the caret would jump to the end.
    requestAnimationFrame(() => {
      area.focus();
      area.setSelectionRange(next.start, next.end);
    });
  };

  const save = async () => {
    if (!text.trim()) {
      setError("Type something to save.");
      return;
    }
    setSaving(true);
    try {
      await createTextClip(text);
      await getCurrentWindow().close();
    } catch (e) {
      setError(errorMessage(e));
      setSaving(false);
    }
  };

  return (
    <div className="flex h-full flex-col bg-panel">
      <header className="shrink-0 border-b border-edge px-3 py-2">
        <h1 className="text-[13px] font-semibold text-ink">New clip</h1>
        <p className="mt-0.5 text-[11px] text-ink-faint">
          Markdown formatting is saved as Markdown; everything else stays plain text.
        </p>
      </header>

      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-edge bg-panel-2/50 px-3 py-1.5">
        {BUTTONS.map((button) => (
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

      <textarea
        ref={areaRef}
        autoFocus
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
            event.preventDefault();
            void save();
          }
        }}
        spellCheck={false}
        placeholder="Type or paste the text for the new clip…"
        aria-label="Clip text"
        className="min-h-0 flex-1 resize-none border-0 bg-transparent px-3 py-2 text-[12px] leading-relaxed text-ink placeholder:text-ink-faint focus:outline-none"
      />

      {error && (
        <p role="alert" className="shrink-0 border-t border-danger/30 bg-danger/10 px-3 py-1.5 text-[11px] text-danger">
          {error}
        </p>
      )}

      <footer className="flex shrink-0 items-center gap-2 border-t border-edge px-3 py-2">
        <span className="mr-auto text-[10px] text-ink-faint">Ctrl+Enter to save</span>
        <button
          type="button"
          onClick={close}
          className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={() => void save()}
          disabled={saving}
          className="rounded border border-accent bg-accent px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-60"
        >
          {saving ? "Saving…" : "Save clip"}
        </button>
      </footer>
    </div>
  );
}
