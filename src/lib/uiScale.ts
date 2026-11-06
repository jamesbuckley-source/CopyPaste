/**
 * The interface scale the whole interface is drawn at.
 *
 * `#root` carries `zoom: var(--ui-scale)`, so every length inside it is multiplied
 * by this while `window.innerWidth` and pointer coordinates are not. Anything that
 * measures the page, or turns pointer travel into a change of width, has to divide
 * by it or it comes out that much too big at any text scale above 100%.
 */
export function rootScale(): number {
  const value = Number(
    getComputedStyle(document.documentElement).getPropertyValue("--ui-scale"),
  );
  return Number.isFinite(value) && value > 0 ? value : 1;
}
