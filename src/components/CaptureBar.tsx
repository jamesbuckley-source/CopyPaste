import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";

import {
  IconCheck,
  IconChevronDown,
  IconEyedropper,
  IconFullScreenCapture,
  IconRecord,
  IconRegionCapture,
  IconRuler,
  IconScrollCapture,
  IconWindowCapture,
} from "./icons";
import type { CaptureMode } from "../lib/types";

interface CaptureBarProps {
  /**
   * `row` is the sidebar's top strip; `column` is the collapsed rail, which is
   * too narrow for a row.
   */
  orientation?: "row" | "column";
  /** The capture type the split button repeats and the menu ticks. */
  captureMode: CaptureMode;
  onRememberCaptureMode: (mode: CaptureMode) => void;
  onPickColor: () => void;
  onCaptureRegion: () => void;
  onCaptureWindow: () => void;
  onCaptureScreen: () => void;
  onCaptureScrolling: () => void;
  /** Hides the window and opens the onscreen ruler. */
  onOpenRuler: () => void;
  /** Opens the recording toolbar, where a recording is chosen and started. */
  onRecord: () => void;
}

/**
 * Capture tools for the history sidebar.
 *
 * The four capture kinds share one split button, exactly as they do in the title
 * bar: the wide half repeats the kind last used and the chevron opens all four
 * with the current one ticked. The strip is 186px wide and has to hold the
 * eyedropper, the ruler, the split button and record, so four separate icons for
 * the capture kinds would cost more room than the job is worth.
 */
export function CaptureBar({
  orientation = "row",
  captureMode,
  onRememberCaptureMode,
  onPickColor,
  onCaptureRegion,
  onCaptureWindow,
  onCaptureScreen,
  onCaptureScrolling,
  onOpenRuler,
  onRecord,
}: CaptureBarProps) {
  const column = orientation === "column";
  const [menuOpen, setMenuOpen] = useState(false);
  /** Where the fly-out sits, in viewport coordinates; null until it is measured. */
  const [menuAt, setMenuAt] = useState<{ left: number; top: number } | null>(null);
  const groupRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const MENU_WIDTH = 224;

  const modes = [
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
      label: "Scroll and capture a window",
      Icon: IconScrollCapture,
      run: onCaptureScrolling,
    },
  ] satisfies { id: CaptureMode; label: string; Icon: typeof IconRegionCapture; run: () => void }[];

  const active = modes.find((mode) => mode.id === captureMode) ?? modes[0];

  // The popup has no backdrop, so an outside click and Escape both have to be
  // heard by the window. Escape returns focus to the trigger, which is where a
  // keyboard user left off.
  useEffect(() => {
    if (!menuOpen) return;

    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      // The menu is portalled out of this subtree, so it has to be excluded by hand.
      if (groupRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setMenuOpen(false);
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
    // The sidebar scrolls, and a fixed popup would stay behind while it did.
    window.addEventListener("scroll", onResize, true);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("scroll", onResize, true);
    };
  }, [menuOpen]);

  /**
   * Places the fly-out.
   *
   * It is rendered into `document.body` rather than inside the sidebar: that column
   * scrolls, and anything absolutely positioned within it is clipped at its edge —
   * which is what made the menu look cut off. Measuring in viewport coordinates and
   * positioning with `fixed` keeps it whole wherever the strip is.
   */
  const openMenu = () => {
    const rect = groupRef.current?.getBoundingClientRect();
    if (!rect) return;
    const left = Math.max(
      4,
      Math.min(
        column ? rect.right + 4 : rect.left,
        window.innerWidth - MENU_WIDTH - 4,
      ),
    );
    const top = Math.max(4, Math.min(rect.bottom + 4, window.innerHeight - 200));
    setMenuAt({ left, top });
    setMenuOpen(true);
  };

  // Focus the entry for the type in use, so the menu opens where the eye already is.
  useEffect(() => {
    if (!menuOpen) return;
    const current = menuRef.current?.querySelector<HTMLButtonElement>('[aria-current="true"]');
    current?.focus();
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

  const choose = (mode: (typeof modes)[number]) => {
    setMenuOpen(false);
    // Remembered first, so a capture that hides the window records the choice.
    onRememberCaptureMode(mode.id);
    mode.run();
  };

  return (
    <div
      role="toolbar"
      aria-label="Capture tools"
      aria-orientation={column ? "vertical" : undefined}
      className={`flex ${
        column ? "flex-col items-center gap-1" : "items-center gap-1 pl-2 pr-6 pt-1.5"
      }`}
    >
      <IconButton label="Pick a color from the screen" onClick={onPickColor}>
        <IconEyedropper className="size-3.5" />
      </IconButton>

      <IconButton label="Measure on screen" onClick={onOpenRuler}>
        <IconRuler className="size-3.5" />
      </IconButton>

      <Divider column={column} />

      {column ? (
        modes.map((mode) => {
          const ModeIcon = mode.Icon;
          return (
            <IconButton
              key={mode.id}
              label={mode.label}
              onClick={() => {
                onRememberCaptureMode(mode.id);
                mode.run();
              }}
            >
              <ModeIcon className="size-3.5" />
            </IconButton>
          );
        })
      ) : (
        <div ref={groupRef} className="relative flex items-center" onKeyDown={onMenuKeyDown}>
        <button
          type="button"
          onClick={() => {
            setMenuOpen(false);
            active.run();
          }}
          title={active.label}
          aria-label={active.label}
          className="flex size-5 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <active.Icon className="size-3.5" />
        </button>
        <button
          ref={triggerRef}
          type="button"
          onClick={() => (menuOpen ? setMenuOpen(false) : openMenu())}
          title="Choose a capture type"
          aria-label="Choose a capture type"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className="flex h-5 w-3 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconChevronDown className="size-2.5" />
        </button>

        {menuOpen &&
          menuAt &&
          createPortal(
            <div
              ref={menuRef}
              role="menu"
              aria-label="Capture types"
              style={{ left: menuAt.left, top: menuAt.top, width: MENU_WIDTH }}
              onKeyDown={onMenuKeyDown}
              className="fixed z-50 rounded border border-edge bg-panel py-1 shadow-xl animate-fade-in"
            >
              {modes.map((mode) => {
                const ModeIcon = mode.Icon;
                return (
                  <button
                    key={mode.id}
                    type="button"
                    role="menuitem"
                    aria-current={mode.id === active.id ? "true" : undefined}
                    onClick={() => choose(mode)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] leading-snug text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink focus:outline-none focus-visible:bg-panel-2 focus-visible:text-ink"
                  >
                    <span className="flex size-3.5 shrink-0 items-center justify-center text-ink-faint">
                      <ModeIcon className="size-3.5" />
                    </span>
                    <span className="min-w-0 flex-1">{mode.label}</span>
                    {mode.id === active.id && (
                      <IconCheck className="size-3 shrink-0 text-accent" />
                    )}
                  </button>
                );
              })}
            </div>,
            document.body,
          )}
        </div>
      )}

      <IconButton label="Record the screen" onClick={onRecord}>
        <IconRecord className="size-3.5" />
      </IconButton>
    </div>
  );
}

function Divider({ column }: { column: boolean }) {
  return (
    <span
      className={`shrink-0 bg-edge ${column ? "my-0.5 h-px w-3.5" : "mx-0.5 h-3.5 w-px"}`}
      aria-hidden="true"
    />
  );
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className="flex size-5 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
    >
      {children}
    </button>
  );
}
