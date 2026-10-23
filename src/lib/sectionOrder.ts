import { useCallback, useMemo, useState, type DragEvent } from "react";

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
 * Drag-to-reorder state for a run of sections. The DOM order never changes — the
 * sections are laid out with CSS `order` — so dragging cannot unmount a section
 * and lose a half-typed collection name.
 */
export function useSectionOrder(
  stored: readonly string[],
  ids: readonly string[],
  onChange: (order: string[]) => void,
) {
  const order = useMemo(() => applyOrder(stored, ids), [stored, ids.join("|")]);
  const [dragging, setDragging] = useState<string | null>(null);

  const rankOf = useCallback((id: string) => {
    const at = order.indexOf(id);
    return at === -1 ? order.length : at;
  }, [order]);

  const dragProps = useCallback(
    (id: string) => ({
      draggable: true,
      onDragStart: (event: DragEvent<HTMLElement>) => {
        event.dataTransfer.effectAllowed = "move";
        setDragging(id);
      },
      onDragOver: (event: DragEvent<HTMLElement>) => {
        if (dragging && dragging !== id) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
        }
      },
      onDrop: (event: DragEvent<HTMLElement>) => {
        event.preventDefault();
        if (dragging) onChange(moveBefore(order, dragging, id));
        setDragging(null);
      },
      onDragEnd: () => setDragging(null),
    }),
    [dragging, onChange, order],
  );

  return { rankOf, dragProps, dragging };
}

export type SectionDragProps = ReturnType<ReturnType<typeof useSectionOrder>["dragProps"]>;
