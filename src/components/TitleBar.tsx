import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { CaptureMode } from "../lib/types";
import {
  IconCheck,
  IconChevronDown,
  IconClose,
  IconEyedropper,
  IconFullScreenCapture,
  IconMaximize,
  IconMinimize,
  IconRegionCapture,
  IconRestore,
  IconScrollCapture,
  IconSettings,
  IconWindowCapture,
} from "./icons";

interface TitleBarProps {
  subtitle: string;
  settingsOpen: boolean;
  onToggleSettings: () => void;
  onPickColor: () => void;
  /** Screenshot mode the header button offers and the menu remembers. */
  captureMode: CaptureMode;
  onRememberCaptureMode: (mode: CaptureMode) => void;
  onCaptureRegion: () => void;
  onCaptureWindow: () => void;
  onCaptureScreen: () => void;
  onCaptureScrolling: () => void;
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
  captureMode,
  onRememberCaptureMode,
  onCaptureRegion,
  onCaptureWindow,
  onCaptureScreen,
  onCaptureScrolling,
  onHide,
}: TitleBarProps) {
  const [maximized, setMaximized] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const groupRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const captureModes = [
    { id: "region", label: "Capture a region", Icon: IconRegionCapture, run: onCaptureRegion },
    { id: "window", label: "Capture a window", Icon: IconWindowCapture, run: onCaptureWindow },
    {
      id: "screen",
      label: "Capture the whole screen",
      Icon: IconFullScreenCapture,
      run: onCaptureScreen,
    },
    {
      id: "scrolling",
      label: "Scroll a window and capture its full length",
      Icon: IconScrollCapture,
      run: onCaptureScrolling,
    },
  ] satisfies { id: CaptureMode; label: string; Icon: typeof IconRegionCapture; run: () => void }[];

  const activeMode = captureModes.find((mode) => mode.id === captureMode) ?? captureModes[0];

  // The popup has no backdrop, so an outside click and Escape both have to be
  // heard by the window. Escape returns focus to the trigger, which is where a
  // keyboard user left off.
  useEffect(() => {
    if (!menuOpen) return;

    const onPointerDown = (event: MouseEvent) => {
      if (!groupRef.current?.contains(event.target as Node)) setMenuOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setMenuOpen(false);
      triggerRef.current?.focus();
    };
    const onResize = () => setMenuOpen(false);

    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onResize);
    };
  }, [menuOpen]);

  useEffect(() => {
    if (!menuOpen) return;
    const items = menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
    const current = menuRef.current?.querySelector<HTMLButtonElement>('[aria-current="true"]');
    (current ?? items?.[0])?.focus();
  }, [menuOpen]);

  const onMenuKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [],
    );
    if (items.length === 0) return;
    event.preventDefault();
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const step = event.key === "ArrowDown" ? 1 : -1;
    const next =
      index < 0 ? (step > 0 ? 0 : items.length - 1) : (index + step + items.length) % items.length;
    items[next].focus();
  };

  const chooseMode = (mode: (typeof captureModes)[number]) => {
    setMenuOpen(false);
    // Remember first, so a capture that hides the window records the choice.
    onRememberCaptureMode(mode.id);
    mode.run();
  };

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

      <span className="h-4 w-px shrink-0 bg-edge" aria-hidden="true" />

      <div ref={groupRef} className="relative flex items-center" onKeyDown={onMenuKeyDown}>
        <button
          type="button"
          onClick={() => {
            setMenuOpen(false);
            activeMode.run();
          }}
          title={activeMode.label}
          aria-label={activeMode.label}
          className="flex size-6 items-center justify-center rounded text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <activeMode.Icon className="size-3.5" />
        </button>

        <button
          ref={triggerRef}
          type="button"
          onClick={() => setMenuOpen((open) => !open)}
          title="Choose a capture type"
          aria-label="Choose a capture type"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className="flex h-6 w-4 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconChevronDown className="size-2.5" />
        </button>

        {menuOpen && (
          <div
            ref={menuRef}
            role="menu"
            aria-label="Capture types"
            className="absolute right-0 top-full z-50 mt-1 w-60 rounded border border-edge bg-panel py-1 shadow-xl animate-fade-in"
          >
            {captureModes.map((mode) => {
              const ModeIcon = mode.Icon;
              return (
                <button
                  key={mode.id}
                  type="button"
                  role="menuitem"
                  aria-current={mode.id === activeMode.id ? "true" : undefined}
                  onClick={() => chooseMode(mode)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] leading-snug text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink focus:outline-none focus-visible:bg-panel-2 focus-visible:text-ink"
                >
                  <span className="flex size-3.5 shrink-0 items-center justify-center text-ink-faint">
                    <ModeIcon className="size-3.5" />
                  </span>
                  <span className="min-w-0 flex-1">{mode.label}</span>
                  {mode.id === activeMode.id && (
                    <IconCheck className="size-3 shrink-0 text-accent" />
                  )}
                </button>
              );
            })}
          </div>
        )}
      </div>

      <span className="h-4 w-px shrink-0 bg-edge" aria-hidden="true" />

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
