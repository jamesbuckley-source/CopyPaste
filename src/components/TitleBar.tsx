import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  IconClose,
  IconEyedropper,
  IconMaximize,
  IconMinimize,
  IconRegionCapture,
  IconRestore,
  IconSettings,
  IconWindowCapture,
} from "./icons";

interface TitleBarProps {
  subtitle: string;
  settingsOpen: boolean;
  onToggleSettings: () => void;
  onPickColor: () => void;
  onCaptureRegion: () => void;
  onCaptureWindow: () => void;
  onHide: () => void;
}

/**
 * Frameless-window header with window controls.
 *
 * `data-tauri-drag-region` is what makes a decorations-free window movable, and it
 * applies only to the element carrying it — so every non-interactive child gets the
 * attribute too. Otherwise dragging the title text or the subtitle would do nothing
 * because the pointer lands on a child element.
 */
export function TitleBar({
  subtitle,
  settingsOpen,
  onToggleSettings,
  onPickColor,
  onCaptureRegion,
  onCaptureWindow,
  onHide,
}: TitleBarProps) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let cancelled = false;

    const sync = () => {
      appWindow
        .isMaximized()
        .then((value) => {
          if (!cancelled) setMaximized(value);
        })
        .catch(() => undefined);
    };

    sync();
    // Dragging to the screen edge maximises too, so the icon has to follow the
    // real window state rather than only our own button clicks.
    const pending = appWindow.onResized(sync);
    return () => {
      cancelled = true;
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  return (
    <header
      data-tauri-drag-region
      className="flex shrink-0 items-center gap-2.5 border-b border-edge bg-panel py-2 pl-3 pr-1.5"
    >
      <div
        data-tauri-drag-region
        className="flex size-5 shrink-0 items-center justify-center rounded bg-accent/15 text-accent"
      >
        <svg viewBox="0 0 16 16" className="size-3.5" aria-hidden="true">
          <rect x="3" y="2.5" width="8.5" height="11" rx="1.6" fill="currentColor" opacity="0.5" />
          <rect x="5" y="4.5" width="8.5" height="9.5" rx="1.6" fill="currentColor" />
        </svg>
      </div>

      <h1 data-tauri-drag-region className="text-[12px] font-semibold tracking-wide text-ink">
        CopyPaste
      </h1>

      <span
        data-tauri-drag-region
        className="min-w-0 flex-1 truncate text-[11px] text-ink-faint"
      >
        {subtitle}
      </span>

      <button
        type="button"
        onClick={onPickColor}
        title="Pick a color from the screen"
        aria-label="Pick a color from the screen"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
      >
        <IconEyedropper className="size-3.5" />
      </button>

      <button
        type="button"
        onClick={onCaptureRegion}
        title="Capture a region"
        aria-label="Capture a region"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
      >
        <IconRegionCapture className="size-3.5" />
      </button>

      <button
        type="button"
        onClick={onCaptureWindow}
        title="Capture a window"
        aria-label="Capture a window"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
      >
        <IconWindowCapture className="size-3.5" />
      </button>

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

      <span className="mx-0.5 h-4 w-px shrink-0 bg-edge" aria-hidden="true" />

      <button
        type="button"
        onClick={() => void getCurrentWindow().minimize()}
        title="Minimize"
        aria-label="Minimize"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
      >
        <IconMinimize className="size-3.5" />
      </button>

      <button
        type="button"
        onClick={() => void getCurrentWindow().toggleMaximize()}
        title={maximized ? "Restore" : "Maximize"}
        aria-label={maximized ? "Restore" : "Maximize"}
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
      >
        {maximized ? (
          <IconRestore className="size-3.5" />
        ) : (
          <IconMaximize className="size-3.5" />
        )}
      </button>

      <button
        type="button"
        onClick={onHide}
        title="Close to tray (Esc)"
        aria-label="Close to tray"
        className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-danger/20 hover:text-danger"
      >
        <IconClose className="size-3.5" />
      </button>
    </header>
  );
}
