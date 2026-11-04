/**
 * The countdown shown before a delayed screenshot.
 *
 * A window of its own, parked in the middle of the primary screen: the capture
 * overlay has not opened yet, so this is the only thing on screen and it has to
 * both announce the delay and get out of the way. The backend owns the clock and
 * closes the window when the delay is up; this counts down the seconds for the
 * reading and cancels the delay if `Esc` is pressed first.
 */

import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { useSettings } from "../hooks/useSettings";
import { cancelCaptureDelay } from "../lib/ipc";

/** The delay the window was opened for, taken from its URL. */
function requestedSeconds(): number {
  const value = Number(new URLSearchParams(window.location.search).get("seconds"));
  return Number.isFinite(value) && value > 0 ? Math.round(value) : 5;
}

export function CaptureCountdown() {
  const [left, setLeft] = useState(requestedSeconds);
  const { settings } = useSettings();
  // The ring is drawn in the same colour as the capture outlines, so the thing that
  // is counting down and the frame that follows it read as one tool.
  const ring = settings?.captureOutline ?? "#d60000";

  // The app paints an opaque canvas behind its panels, and this window is a ring:
  // without this the corners outside the circle would be a square of colour.
  useEffect(() => {
    const { body, documentElement } = document;
    const previous = [body.style.background, documentElement.style.background];
    body.style.background = "transparent";
    documentElement.style.background = "transparent";
    return () => {
      body.style.background = previous[0];
      documentElement.style.background = previous[1];
    };
  }, []);

  const cancel = useCallback(() => {
    // Told first: the capture thread is waiting on this flag, and closing the
    // window alone would leave it counting down to a capture nobody wants.
    cancelCaptureDelay()
      .catch(() => undefined)
      .finally(() => {
        void getCurrentWindow()
          .close()
          .catch(() => undefined);
      });
  }, []);

  useEffect(() => {
    const tick = window.setInterval(() => setLeft((value) => Math.max(0, value - 1)), 1000);
    return () => window.clearInterval(tick);
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel]);

  return (
    <div
      className="flex h-full w-full cursor-default flex-col items-center justify-center rounded-full border-2 bg-panel/95 shadow-xl backdrop-blur"
      style={{ borderColor: ring }}
      onClick={cancel}
      title="Cancel the delay"
    >
      <span
        className="font-mono text-[64px] leading-none tabular-nums text-ink"
        style={{ color: ring }}
        aria-live="polite"
      >
        {left}
      </span>
      <span className="mt-2 text-[11px] text-ink-faint">Esc cancels</span>
    </div>
  );
}
