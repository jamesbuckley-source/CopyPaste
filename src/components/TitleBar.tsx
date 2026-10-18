import { IconClose, IconSettings } from "./icons";

interface TitleBarProps {
  subtitle: string;
  settingsOpen: boolean;
  onToggleSettings: () => void;
  onHide: () => void;
}

/**
 * Frameless-window header.
 *
 * `data-tauri-drag-region` is what makes a decorations-free window movable; only
 * elements carrying the attribute start a drag, so the buttons below stay
 * clickable.
 */
export function TitleBar({
  subtitle,
  settingsOpen,
  onToggleSettings,
  onHide,
}: TitleBarProps) {
  return (
    <header
      data-tauri-drag-region
      className="flex shrink-0 items-center gap-2.5 border-b border-edge bg-panel px-3 py-2"
    >
      <div className="flex size-5 shrink-0 items-center justify-center rounded bg-accent/15 text-accent">
        <svg viewBox="0 0 16 16" className="size-3.5" aria-hidden="true">
          <rect x="3" y="2.5" width="8.5" height="11" rx="1.6" fill="currentColor" opacity="0.5" />
          <rect x="5" y="4.5" width="8.5" height="9.5" rx="1.6" fill="currentColor" />
        </svg>
      </div>

      <h1 className="text-[12px] font-semibold tracking-wide text-ink">CopyPaste</h1>

      <span className="min-w-0 flex-1 truncate text-[11px] text-ink-faint">{subtitle}</span>

      <button
        type="button"
        onClick={onToggleSettings}
        title={settingsOpen ? "Back to history" : "Settings"}
        aria-label={settingsOpen ? "Back to history" : "Settings"}
        aria-pressed={settingsOpen}
        className={`flex size-6 items-center justify-center rounded transition-colors ${
          settingsOpen
            ? "bg-accent/20 text-accent-hi"
            : "text-ink-dim hover:bg-panel-3 hover:text-ink"
        }`}
      >
        <IconSettings className="size-3.5" />
      </button>

      <button
        type="button"
        onClick={onHide}
        title="Hide window (Esc)"
        aria-label="Hide window"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-danger/20 hover:text-danger"
      >
        <IconClose className="size-3.5" />
      </button>
    </header>
  );
}
