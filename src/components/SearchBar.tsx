import type { RefObject } from "react";

import { IconClose, IconSearch } from "./icons";

interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  inputRef: RefObject<HTMLInputElement | null>;
  busy: boolean;
  resultCount: number;
  activeFilter: string;
  onClearFilter: () => void;
}

export function SearchBar({
  value,
  onChange,
  inputRef,
  busy,
  resultCount,
  activeFilter,
  onClearFilter,
}: SearchBarProps) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-edge bg-panel px-3 py-2">
      <IconSearch className="size-3.5 shrink-0 text-ink-faint" />

      <input
        ref={inputRef}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        // The window is summoned to be typed into, so it should never open with
        // the caret somewhere else.
        autoFocus
        spellCheck={false}
        autoComplete="off"
        placeholder="Search clips…"
        aria-label="Search clips"
        className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-faint focus:outline-none"
      />

      {activeFilter !== "All clips" && (
        <button
          type="button"
          onClick={onClearFilter}
          title="Clear the active filter"
          className="flex items-center gap-1 rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-medium text-accent-hi transition-colors hover:bg-accent/25"
        >
          {activeFilter}
          <IconClose className="size-2.5" />
        </button>
      )}

      <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">
        {busy ? "…" : `${resultCount}`}
      </span>
    </div>
  );
}
