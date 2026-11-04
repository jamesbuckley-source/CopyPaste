/**
 * "Capture Delay Settings…" in the screenshot menu, and the dialog behind it.
 *
 * The delay lives in the menu rather than in the toolbar: it is set once and left
 * alone for weeks, so it belongs with the rest of the screenshot choices instead of
 * taking up a button beside the ones that are pressed every day. The menu row shows
 * the current value, which is the only feedback the setting needs.
 */

import { useSettings } from "../hooks/useSettings";
import { clampDelay } from "../lib/captureDelay";
import { IconClock } from "./icons";
import { PromptDialog } from "./PromptDialog";

/** The menu row. `onOpen` closes the menu and raises the dialog. */
export function CaptureDelayMenuItem({ onOpen }: { onOpen: () => void }) {
  const { settings } = useSettings();
  const delay = settings?.captureDelaySecs ?? 0;

  return (
    <button
      type="button"
      role="menuitem"
      onClick={onOpen}
      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] leading-snug text-ink-dim transition-colors hover:bg-panel-2 hover:text-ink focus:outline-none focus-visible:bg-panel-2 focus-visible:text-ink"
    >
      <span className="flex size-3.5 shrink-0 items-center justify-center text-ink-faint">
        <IconClock className="size-3.5" />
      </span>
      <span className="min-w-0 flex-1">Capture Delay Settings…</span>
      <span className="shrink-0 text-[10px] tabular-nums text-ink-faint">
        {delay > 0 ? `${delay}s` : "Off"}
      </span>
    </button>
  );
}

/** The dialog: seconds to count down, with 0 for no delay. */
export function CaptureDelayDialog({ onClose }: { onClose: () => void }) {
  const { settings, patch } = useSettings();
  const delay = settings?.captureDelaySecs ?? 0;

  const apply = (value: string) => {
    const seconds = Math.round(Number(value));
    onClose();
    void patch({ captureDelaySecs: Number.isFinite(seconds) ? clampDelay(seconds) : 0 });
  };

  return (
    <PromptDialog
      title="Capture delay"
      description="Seconds to count down after the area is chosen, before the picture is taken. Use it to open a menu or hover a tooltip — the desktop stays yours while it counts. 0 takes screenshots straight away, and Esc during a countdown cancels that shot."
      label="Seconds"
      initialValue={String(delay)}
      placeholder="5"
      confirmLabel="Set delay"
      choices={[
        { label: "No delay", value: "0" },
        { label: "3 seconds", value: "3" },
        { label: "5 seconds", value: "5" },
        { label: "10 seconds", value: "10" },
      ]}
      onCancel={onClose}
      onSubmit={apply}
    />
  );
}
