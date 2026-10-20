import { formatBytes, formatCount } from "../lib/format";
import type { Settings, Stats } from "../lib/types";
import { IconClock, IconPause, IconPlay } from "./icons";

interface StatusBarProps {
  stats: Stats | null;
  settings: Settings | null;
  /** Running app version; the button checks for a newer release. */
  version: string | null;
  /** True while an update check is in flight. */
  checkingForUpdates: boolean;
  onCheckForUpdates: () => void;
  shown: number;
  total: number;
  /** Clips picked for a bulk action. */
  picked: number;
  /** Unix-millisecond deadline of a timed pause. */
  pausedUntil: number | null;
  pausedIndefinitely: boolean;
  onResume: () => void;
  now: number;
}

export function StatusBar({
  stats,
  settings,
  version,
  checkingForUpdates,
  onCheckForUpdates,
  shown,
  total,
  picked,
  pausedUntil,
  pausedIndefinitely,
  onResume,
  now,
}: StatusBarProps) {
  const truncated = total > shown;
  const remainingMinutes =
    pausedUntil !== null ? Math.max(1, Math.round((pausedUntil - now) / 60_000)) : 0;
  const paused = pausedIndefinitely || remainingMinutes > 0;

  return (
    <footer className="flex shrink-0 items-center gap-3 border-t border-edge bg-panel px-3 py-1.5 text-[10px] text-ink-faint">
      <span className="tabular-nums">
        {picked > 1
          ? `${picked} selected`
          : truncated
            ? `Showing ${shown} of ${formatCount(total, "clip")}`
            : formatCount(total, "clip")}
      </span>

      {stats && stats.pinned > 0 && <span className="tabular-nums">{stats.pinned} pinned</span>}

      {stats && stats.mediaBytes > 0 && (
        <span className="tabular-nums">{formatBytes(stats.mediaBytes)} of images</span>
      )}

      <span className="flex-1" />

      {paused && (
        <button
          type="button"
          onClick={onResume}
          title="Capture is paused — click to start recording again"
          className="flex items-center gap-1 rounded bg-warn/15 px-1.5 py-0.5 text-[10px] font-medium text-warn transition-colors hover:bg-warn/25"
        >
          {pausedIndefinitely ? (
            <IconPause className="size-3" />
          ) : (
            <IconClock className="size-3" />
          )}
          {pausedIndefinitely ? "Capture paused" : `Paused ${remainingMinutes}m`}
          <IconPlay className="size-3" />
        </button>
      )}

      <button
        type="button"
        onClick={onCheckForUpdates}
        disabled={checkingForUpdates}
        title="Check for updates"
        aria-label="Check for updates"
        className="rounded px-1.5 py-0.5 tabular-nums text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-60"
      >
        {checkingForUpdates ? "Checking…" : version ? `v${version}` : "…"}
      </button>

      {settings && (
        <>
          <span className="text-edge">|</span>
          <span className="tabular-nums">{settings.hotkey}</span>
        </>
      )}
    </footer>
  );
}
