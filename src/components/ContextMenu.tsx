/**
 * A small right-click menu.
 *
 * `#root` is scaled with CSS `zoom`, so pointer coordinates from the mouse event
 * have to be divided by the interface scale before they can position a fixed
 * element inside the zoomed tree.
 */

import { useEffect } from "react";

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
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
      {items.map((item) => (
        <div key={item.label}>
          {item.separatorBefore && <div className="my-1 h-px bg-edge-soft" aria-hidden="true" />}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              item.onSelect();
              onClose();
            }}
            className="block w-full px-3 py-1 text-left text-[11px] text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink"
          >
            {item.label}
          </button>
        </div>
      ))}
    </div>
  );
}
