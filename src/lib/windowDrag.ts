/**
 * Moving a frameless window the way a title bar does everywhere else.
 *
 * Tauri's `data-tauri-drag-region` hands the move to Windows on the first
 * `mousedown`, which enters the native move loop before the pointer has travelled a
 * pixel. Windows then paints the window in its "being moved" state — translucent —
 * so what turns out to be a click flashes the whole window. Native applications do
 * not start the move until the pointer has passed the small distance Windows calls
 * the drag threshold (`SM_CXDRAG`, four pixels by default), and neither does this.
 *
 * Any element marked `data-drag-window` is a handle; controls inside one are not.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

/** The attribute that marks a handle. Deliberately not Tauri's own. */
export const DRAG_ATTRIBUTE = "data-drag-window";

/** Pixels of travel before the move starts; Windows' own default for `SM_CXDRAG`. */
const DRAG_THRESHOLD = 4;

/** Anything that is meant to be clicked rather than used to move the window. */
const CONTROL = "button, a, input, select, textarea, label, [role='menuitem']";

let installed = false;

/**
 * Starts listening for drags. Called once, from the entry point, so every window
 * the app opens — the overlay, the recording bar, sticky clips, the stack panel —
 * moves the same way.
 */
export function installWindowDrag(): void {
  if (installed) return;
  installed = true;
  // Capture phase: the move has to be claimed before anything inside the handle
  // reacts to the press.
  window.addEventListener("mousedown", onMouseDown, true);
  window.addEventListener("dblclick", onDoubleClick, true);
}

/** The handle a pointer event landed on, if it landed on one. */
function handleAt(target: EventTarget | null): Element | null {
  if (!(target instanceof Element)) return null;
  if (target.closest(CONTROL)) return null;
  return target.closest(`[${DRAG_ATTRIBUTE}]`);
}

function onMouseDown(event: MouseEvent): void {
  if (event.button !== 0 || event.detail > 1) return;
  if (!handleAt(event.target)) return;

  const startX = event.screenX;
  const startY = event.screenY;
  let started = false;

  const stop = () => {
    window.removeEventListener("mousemove", onMove, true);
    window.removeEventListener("mouseup", stop, true);
  };

  const onMove = (move: MouseEvent) => {
    // Screen coordinates are unaffected by the interface `zoom`, so the threshold
    // is the same real distance at every text scale.
    const travelled = Math.hypot(move.screenX - startX, move.screenY - startY);
    if (started || travelled < DRAG_THRESHOLD) return;
    started = true;
    stop();
    // From here the OS runs the move, and the pointer leaves our hands until the
    // button comes up.
    void getCurrentWindow().startDragging();
  };

  window.addEventListener("mousemove", onMove, true);
  window.addEventListener("mouseup", stop, true);
}

/** Double-clicking a title bar maximises it, as it always has. */
function onDoubleClick(event: MouseEvent): void {
  if (!handleAt(event.target)) return;
  void getCurrentWindow().toggleMaximize();
}
