import { Fragment } from "react";

import {
  IconEyedropper,
  IconFullScreenCapture,
  IconRegionCapture,
  IconScrollCapture,
  IconWindowCapture,
} from "./icons";

interface CaptureBarProps {
  /**
   * `row` is the sidebar's top strip; `column` is the collapsed rail, which is
   * too narrow for a row.
   */
  orientation?: "row" | "column";
  onPickColor: () => void;
  onCaptureRegion: () => void;
  onCaptureWindow: () => void;
  onCaptureScreen: () => void;
  onCaptureScrolling: () => void;
}

/**
 * Capture tools for the history sidebar.
 *
 * Deliberately plain buttons rather than the title bar's split button: running
 * one of these here must not change the capture type the title bar remembers.
 * The sidebar is 186px wide, so the buttons are icon-only and the tooltip and
 * accessible name carry the wording.
 */
export function CaptureBar({
  orientation = "row",
  onPickColor,
  onCaptureRegion,
  onCaptureWindow,
  onCaptureScreen,
  onCaptureScrolling,
}: CaptureBarProps) {
  const column = orientation === "column";

  const actions = [
    { label: "Pick a color from the screen", Icon: IconEyedropper, run: onPickColor },
    { label: "Capture a region", Icon: IconRegionCapture, run: onCaptureRegion },
    { label: "Capture a window", Icon: IconWindowCapture, run: onCaptureWindow },
    { label: "Capture the whole screen", Icon: IconFullScreenCapture, run: onCaptureScreen },
    {
      label: "Scroll and capture a window",
      Icon: IconScrollCapture,
      run: onCaptureScrolling,
    },
  ] satisfies { label: string; Icon: typeof IconRegionCapture; run: () => void }[];

  return (
    <div
      role="toolbar"
      aria-label="Capture tools"
      aria-orientation={column ? "vertical" : undefined}
      className={`flex ${
        column ? "flex-col items-center gap-1" : "items-center gap-1 pl-2 pr-6 pt-1.5"
      }`}
    >
      {actions.map(({ label, Icon, run }, index) => (
        <Fragment key={label}>
          {index === 1 && (
            <span
              className={`shrink-0 bg-edge ${
                column ? "my-0.5 h-px w-3.5" : "mx-0.5 h-3.5 w-px"
              }`}
              aria-hidden="true"
            />
          )}
          <button
            type="button"
            onClick={run}
            title={label}
            aria-label={label}
            className="flex size-5 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
          >
            <Icon className="size-3.5" />
          </button>
        </Fragment>
      ))}
    </div>
  );
}
