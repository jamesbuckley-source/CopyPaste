import { formatBytes, formatCount } from "../lib/format";
import type { Settings, Stats } from "../lib/types";
import { IconShield } from "./icons";

interface StatusBarProps {
  stats: Stats | null;
  settings: Settings | null;
  shown: number;
  total: number;
}

export function StatusBar({ stats, settings, shown, total }: StatusBarProps) {
  const truncated = total > shown;

  return (
    <footer className="flex shrink-0 items-center gap-3 border-t border-edge bg-panel px-3 py-1.5 text-[10px] text-ink-faint">
      <span className="tabular-nums">
        {truncated
          ? `Showing ${shown} of ${formatCount(total, "clip")}`
          : formatCount(total, "clip")}
      </span>

      {stats && stats.pinned > 0 && <span className="tabular-nums">{stats.pinned} pinned</span>}

      {stats && stats.mediaBytes > 0 && (
        <span className="tabular-nums">{formatBytes(stats.mediaBytes)} of images</span>
      )}

      <span className="flex-1" />

      <span
        className="flex items-center gap-1 text-ok"
        title={stats?.encryption ?? "Encrypted local storage"}
      >
        <IconShield className="size-3" />
        {stats?.encryption ? "Encrypted" : "…"}
      </span>

      {settings && (
        <>
          <span className="text-edge">|</span>
          <span className="tabular-nums">{settings.hotkey}</span>
        </>
      )}
    </footer>
  );
}
