import type { CaptureMode } from "../lib/types";
import { IconFullScreenCapture, IconRegionCapture, IconWindowCapture } from "./icons";

/**
 * The capture types a keyboard shortcut or the Print Screen key can start.
 *
 * Scrolling is deliberately absent: it needs a window to be picked and then
 * scrolled for several seconds, which is a poor thing to trigger by accident from
 * a single key. It stays available from the header button and the sidebar strip.
 */
const CHOICES = [
  { value: "screen", label: "Screen", Icon: IconFullScreenCapture },
  { value: "window", label: "Window", Icon: IconWindowCapture },
  { value: "region", label: "Selection", Icon: IconRegionCapture },
] as const satisfies readonly { value: CaptureMode; label: string; Icon: typeof IconRegionCapture }[];

/**
 * Horizontal radio group for the default capture type.
 *
 * A radio group rather than a dropdown because there are only three and the icons
 * carry most of the meaning; the selected one is also the one the header's split
 * button offers and the one Print Screen runs.
 */
export function CaptureModeField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: CaptureMode;
  onChange: (next: CaptureMode) => void;
}) {
  // A stored "scrolling" is not one of the choices, so nothing shows as picked
  // until one is chosen. That is honest: the group only claims what it can offer.
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex items-center gap-1 rounded border border-edge bg-panel-2 p-0.5"
    >
      {CHOICES.map(({ value: mode, label: name, Icon }) => {
        const selected = value === mode;
        return (
          <button
            key={mode}
            type="button"
            role="radio"
            aria-checked={selected}
            title={`${name} capture`}
            onClick={() => onChange(mode)}
            className={`flex items-center gap-1.5 rounded px-2 py-1 text-[11px] transition-colors ${
              selected
                ? "bg-accent/20 text-ink"
                : "text-ink-faint hover:bg-panel-3 hover:text-ink-dim"
            }`}
          >
            <Icon className="size-3.5" />
            {name}
          </button>
        );
      })}
    </div>
  );
}
