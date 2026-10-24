import { useCallback, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

/**
 * Section ids in the order they should be drawn: whatever the user dragged into
 * place first, then anything the stored order does not mention (a section added
 * by a newer build), in the built-in order.
 */
export function applyOrder(stored: readonly string[], ids: readonly string[]): string[] {
  const known = stored.filter((id) => ids.includes(id));
  return [...known, ...ids.filter((id) => !known.includes(id))];
}

/** Moves `id` to sit where `target` currently sits. */
export function moveBefore(order: readonly string[], id: string, target: string): string[] {
  if (id === target) return [...order];
  const next = order.filter((entry) => entry !== id);
  const at = next.indexOf(target);
  if (at === -1) return [...order];
  next.splice(at, 0, id);
  return next;
}

/**
 * Moves `id` so it lands at `index` in the finished order.
 *
 * `index` counts positions in the list *after* the move, so dropping into the last
 * slot is `index === order.length - 1`, which is what an insertion line at the very
 * bottom means.
 */
export function moveTo(order: readonly string[], id: string, index: number): string[] {
  const from = order.indexOf(id);
  if (from === -1) return [...order];
  const next = order.filter((entry) => entry !== id);
  const at = Math.max(0, Math.min(index, next.length));
  next.splice(at, 0, id);
  return next;
}

/** Where a drag would land, given the pointer and the sections in visual order. */
export function dropIndexFor(
  order: readonly string[],
  dragging: string,
  pointerY: number,
  rectOf: (id: string) => { top: number; height: number } | null,
): number {
  const rest = order.filter((id) => id !== dragging);
  for (let index = 0; index < rest.length; index += 1) {
    const rect = rectOf(rest[index]);
    if (!rect) continue;
    // The halfway mark is what makes "before or after this one" feel right: the
    // line follows the pointer rather than waiting for the next section to pass.
    if (pointerY < rect.top + rect.height / 2) return index;
  }
  return rest.length;
}

export interface SectionHandle {
  /** Placement in the user's order; the DOM order never changes. */
  order: number;
  /** Attach to the section's own element so a drag can measure it. */
  register: (element: HTMLElement | null) => void;
  /** Attach to the grip: dragging it re-orders, clicking it does nothing. */
  grip: {
    onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerMove: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerUp: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerCancel: (event: ReactPointerEvent<HTMLElement>) => void;
  };
  /** The insertion line belongs above this section. */
  lineBefore: boolean;
  /** The insertion line belongs below the last section (nothing follows it). */
  lineAfter: boolean;
  dragging: boolean;
}

/**
 * Reordering for a run of sections.
 *
 * Pointer events rather than HTML5 drag and drop: the webview is configured to
 * accept files dragged in from Explorer, and that native handler swallows
 * `dragstart`/`drop` for internal drags, which is why a `draggable` header showed
 * a grab cursor and then did nothing. Pointer events are unaffected by it.
 *
 * The DOM order never changes — the sections are laid out with CSS `order` — so a
 * drag cannot unmount a section and lose a half-typed collection name.
 */
export function useSectionOrder(
  stored: readonly string[],
  ids: readonly string[],
  onChange: (order: string[]) => void,
) {
  const order = useMemo(() => applyOrder(stored, ids), [stored, ids.join("|")]);
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  const elements = useRef(new Map<string, HTMLElement>());
  const active = useRef<{ id: string; pointerId: number } | null>(null);

  const rectOf = useCallback((id: string) => {
    const element = elements.current.get(id);
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return { top: rect.top, height: rect.height };
  }, []);

  const register = useCallback(
    (id: string) => (element: HTMLElement | null) => {
      if (element) elements.current.set(id, element);
      else elements.current.delete(id);
    },
    [],
  );

  const finish = useCallback(
    (commit: boolean) => {
      const drag = active.current;
      active.current = null;
      const target = dropIndex;
      setDragging(null);
      setDropIndex(null);
      if (!commit || !drag || target === null) return;
      const next = moveTo(order, drag.id, target);
      // Nothing moved: a write here would only churn the settings file.
      if (next.join("|") !== order.join("|")) onChange(next);
    },
    [dropIndex, onChange, order],
  );

  const gripProps = useCallback(
    (id: string) => ({
      onPointerDown: (event: ReactPointerEvent<HTMLElement>) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.setPointerCapture(event.pointerId);
        active.current = { id, pointerId: event.pointerId };
        setDragging(id);
        // No movement yet: the line starts where the section already sits.
        setDropIndex(order.indexOf(id));
      },
      onPointerMove: (event: ReactPointerEvent<HTMLElement>) => {
        const drag = active.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        setDropIndex(dropIndexFor(order, drag.id, event.clientY, rectOf));
      },
      onPointerUp: (event: ReactPointerEvent<HTMLElement>) => {
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
        finish(true);
      },
      onPointerCancel: () => finish(false),
    }),
    [finish, order, rectOf],
  );

  const rest = dragging ? order.filter((id) => id !== dragging) : order;

  const handle = useCallback(
    (id: string): SectionHandle => ({
      order: order.indexOf(id),
      register: register(id),
      grip: gripProps(id),
      lineBefore: dragging !== null && dragging !== id && dropIndex === rest.indexOf(id),
      lineAfter:
        dragging !== null &&
        dropIndex === rest.length &&
        rest[rest.length - 1] === id,
      dragging: dragging === id,
    }),
    [dragging, dropIndex, gripProps, order, register, rest],
  );

  return { rankOf: (id: string) => order.indexOf(id), handle, dragging };
}
