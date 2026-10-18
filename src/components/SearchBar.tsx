import type { ReactNode, RefObject } from "react";

import { IconClose, IconGrid, IconList, IconMinus, IconPlus, IconSearch } from "./icons";

export type ViewMode = "list" | "grid";

interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  inputRef: RefObject<HTMLInputElement | null>;
  busy: boolean;
  resultCount: number;
  activeFilter: string;
  onClearFilter: () => void;
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  zoomPercent: number;
  canZoomIn: boolean;
  canZoomOut: boolean;
}

export function SearchBar({
  value,
  onChange,
  inputRef,
  busy,
  resultCount,
  activeFilter,
  onClearFilter,
  viewMode,
  onViewModeChange,
  onZoomIn,
  onZoomOut,
  zoomPercent,
  canZoomIn,
  canZoomOut,
}: SearchBarProps) {
  const segment = (mode: ViewMode, label: string, icon: ReactNode) => (
    <button
      type="button"
      onClick={() => onViewModeChange(mode)}
      title={`${label} view`}
      aria-label={`${label} view`}
      aria-pressed={viewMode === mode}
      className={`flex size-5 items-center justify-center rounded transition-colors ${
        viewMode === mode
          ? "bg-accent/25 text-accent-hi"
          : "text-ink-faint hover:text-ink-dim"
      }`}
    >
      {icon}
    </button>
  );

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
          className="flex shrink-0 items-center gap-1 rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-medium text-accent-hi transition-colors hover:bg-accent/25"
        >
          {activeFilter}
          <IconClose className="size-2.5" />
        </button>
      )}

      <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">
        {busy ? "…" : `${resultCount}`}
      </span>

      <span className="h-4 w-px shrink-0 bg-edge" aria-hidden="true" />

      <div className="flex shrink-0 items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
        {segment("list", "List", <IconList className="size-3" />)}
        {segment("grid", "Card", <IconGrid className="size-3" />)}
      </div>

      {viewMode === "grid" && (
        <div className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            onClick={onZoomOut}
            disabled={!canZoomOut}
            title="Smaller cards"
            aria-label="Smaller cards"
            className="flex size-5 items-center justify-center rounded text-ink-faint transition-colors hover:text-ink disabled:opacity-30"
          >
            <IconMinus className="size-3" />
          </button>
          <span className="w-7 shrink-0 text-center text-[9px] tabular-nums text-ink-faint">
            {zoomPercent}%
          </span>
          <button
            type="button"
            onClick={onZoomIn}
            disabled={!canZoomIn}
            title="Larger cards"
            aria-label="Larger cards"
            className="flex size-5 items-center justify-center rounded text-ink-faint transition-colors hover:text-ink disabled:opacity-30"
          >
            <IconPlus className="size-3" />
          </button>
        </div>
      )}
    </div>
  );
}
