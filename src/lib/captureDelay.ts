/**
 * The screenshot delay in seconds.
 *
 * Zero is off, which is what the clock in the capture strip toggles. The backend
 * clamps the same way, so a number typed into the prompt cannot outlast a minute.
 */

export const MAX_CAPTURE_DELAY = 60;

export function clampDelay(seconds: number): number {
  if (!Number.isFinite(seconds)) return 0;
  return Math.min(Math.max(Math.round(seconds), 0), MAX_CAPTURE_DELAY);
}
