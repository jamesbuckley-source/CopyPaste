import { useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createTextClip, errorMessage } from "../lib/ipc";
import { MarkdownToolbar } from "./MarkdownToolbar";

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

      <MarkdownToolbar
        areaRef={areaRef}
        onChange={setText}
        className="shrink-0 border-b border-edge bg-panel-2/50 px-3 py-1.5"
      />

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
