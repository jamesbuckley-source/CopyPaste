import type { ReactNode, RefObject } from "react";

import type { SortOrder, ViewMode } from "../lib/types";
import { IconClose, IconGrid, IconList, IconSearch } from "./icons";

export type { ViewMode };

const SORT_LABEL: Record<SortOrder, string> = {
  "updated-desc": "Last used ↓",
  "updated-asc": "Last used ↑",
  "created-desc": "Captured ↓",
  "created-asc": "Captured ↑",
};

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
  /** Card size as a percentage, 60 – 200. */
  zoomPercent: number;
  onZoomChange: (percent: number) => void;
  sortOrder: SortOrder;
  onSortChange: (order: SortOrder) => void;
}

/**
 * Drag-to-resize control for the card size.
 *
 * Every movement is reported so the grid resizes under the pointer; the caller
 * debounces persistence so a drag does not write the settings on every pixel.
 */
function ZoomSlider({
  percent,
  onChange,
}: {
  percent: number;
  onChange: (percent: number) => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <input
        type="range"
        min={60}
        max={200}
        step={5}
        value={percent}
        onChange={(event) => onChange(Number(event.target.value))}
        title="Drag to resize the cards"
        aria-label="Card size"
        className="w-20 cursor-pointer accent-accent"
      />
      <span className="w-8 shrink-0 text-center text-[9px] tabular-nums text-ink-faint">
        {percent}%
      </span>
    </div>
  );
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
  zoomPercent,
  onZoomChange,
  sortOrder,
  onSortChange,
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

      <select
        value={sortOrder}
        onChange={(event) => onSortChange(event.target.value as SortOrder)}
        title="Order the history"
        aria-label="Order by"
        className="shrink-0 rounded border border-edge bg-panel-2 px-1 py-0.5 text-[10px] text-ink-dim focus:border-accent-dim focus:outline-none"
      >
        {(Object.keys(SORT_LABEL) as SortOrder[]).map((order) => (
          <option key={order} value={order}>
            {SORT_LABEL[order]}
          </option>
        ))}
      </select>

      <div className="flex shrink-0 items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
        {segment("list", "List", <IconList className="size-3" />)}
        {segment("grid", "Card", <IconGrid className="size-3" />)}
      </div>

      {viewMode === "grid" && <ZoomSlider percent={zoomPercent} onChange={onZoomChange} />}
    </div>
  );
}
