/**
 * A small right-click menu.
 *
 * `#root` is scaled with CSS `zoom`, so pointer coordinates from the mouse event
 * have to be divided by the interface scale before they can position a fixed
 * element inside the zoomed tree.
 */

import { useEffect, useState, type ReactNode } from "react";

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  /** Leading glyph, so a menu entry is recognisable at a glance. */
  icon?: ReactNode;
  /** Items that open in a fly-out when the entry is hovered. */
  submenu?: ContextMenuItem[];
  /** Shows the entry but refuses to run it. */
  disabled?: boolean;
  /** Draws a divider above this entry, to group selection-wide actions. */
  separatorBefore?: boolean;
}

const MENU_WIDTH = 220;

export function ContextMenu({
  x,
  y,
  scale,
  items,
  onClose,
}: {
  x: number;
  y: number;
  scale: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const [openIndex, setOpenIndex] = useState<number | null>(null);

  useEffect(() => {
    const close = () => onClose();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("blur", close);
    };
  }, [onClose]);

  const zoom = scale > 0 ? scale : 1;
  const rowHeight = 26;
  const separatorHeight = 9;
  const estimatedHeight = items.reduce(
    (height, item) => height + rowHeight + (item.separatorBefore ? separatorHeight : 0),
    8,
  );
  const left = Math.max(4, Math.min(x / zoom, window.innerWidth / zoom - MENU_WIDTH - 4));
  const top = Math.max(4, Math.min(y / zoom, window.innerHeight / zoom - estimatedHeight));

  return (
    <div
      role="menu"
      aria-label="Clip actions"
      style={{ left, top, width: MENU_WIDTH }}
      onMouseDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
      className="fixed z-50 rounded border border-edge bg-panel py-1 shadow-xl animate-fade-in"
    >
      {items.map((item, index) => (
        <div
          key={item.label}
          className="relative"
          onMouseEnter={() => setOpenIndex(item.submenu ? index : null)}
          onMouseLeave={() => setOpenIndex(null)}
        >
          {item.separatorBefore && <div className="my-1 h-px bg-edge-soft" aria-hidden="true" />}
          <button
            type="button"
            role="menuitem"
            aria-haspopup={item.submenu ? "menu" : undefined}
            aria-expanded={item.submenu ? openIndex === index : undefined}
            disabled={item.disabled}
            onClick={() => {
              if (item.submenu) {
                setOpenIndex(openIndex === index ? null : index);
                return;
              }
              item.onSelect();
              onClose();
            }}
            className="flex w-full items-center gap-2 px-3 py-1 text-left text-[11px] text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-ink-dim"
          >
            <span className="flex size-3.5 shrink-0 items-center justify-center text-ink-faint">
              {item.icon}
            </span>
            <span className="min-w-0 flex-1 truncate">{item.label}</span>
            {item.submenu && <span className="shrink-0 text-ink-faint">›</span>}
          </button>

          {item.submenu && openIndex === index && (
            <div
              role="menu"
              aria-label={item.label}
              className={`absolute top-0 z-50 max-h-72 w-[190px] overflow-y-auto rounded border border-edge bg-panel py-1 shadow-xl ${
                left + MENU_WIDTH * 2 > window.innerWidth / zoom ? "right-full" : "left-full"
              }`}
            >
              {item.submenu.map((child) => (
                <button
                  key={child.label}
                  type="button"
                  role="menuitem"
                  disabled={child.disabled}
                  onClick={() => {
                    child.onSelect();
                    onClose();
                  }}
                  className="flex w-full items-center gap-2 px-3 py-1 text-left text-[11px] text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-ink-dim"
                >
                  <span className="flex size-3.5 shrink-0 items-center justify-center text-ink-faint">
                    {child.icon}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{child.label}</span>
                </button>
              ))}
              {item.submenu.length === 0 && (
                <p className="px-3 py-1 text-[10px] text-ink-faint">Nothing here yet.</p>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
