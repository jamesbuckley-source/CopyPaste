/**
 * The screen-recording bar.
 *
 * One window serves both halves of the feature: before a recording it is the
 * toolbar — pick what to record, decide whether sound and the pointer are in the
 * shot — and while a recording runs it is the floating controls with the timer,
 * pause and stop. The backend owns the session, so this window only ever reflects
 * `record_status` and asks for changes.
 *
 * The top strip is a drag handle, which is why the close button lives there rather
 * than at the end of the button row: the bar can be moved out of the way of
 * whatever is being recorded, and the two window commands sit together.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";

import {
  IconClose,
  IconFullScreenCapture,
  IconMicrophone,
  IconPause,
  IconPlay,
  IconPointer,
  IconRegionCapture,
  IconSettings,
  IconVideo,
  IconWindowCapture,
} from "./icons";
import { useSettings } from "../hooks/useSettings";
import {
  errorMessage,
  installFfmpeg,
  openSettingsSection,
  recordFixed,
  recordPause,
  recordRegion,
  recordResume,
  recordScreen,
  recordStatus,
  recordStop,
  recordToolchain,
  recordWindow,
} from "../lib/ipc";
import type { RecordStatus, RecordToolchain } from "../lib/types";

/** The preset sizes the fixed-area option offers, plus a free one. */
const FIXED_SIZES = [
  { key: "1920x1080", label: "1920 × 1080", width: 1920, height: 1080 },
  { key: "1280x720", label: "1280 × 720", width: 1280, height: 720 },
  { key: "854x480", label: "854 × 480", width: 854, height: 480 },
] as const;

const WIDTH = 488;
/** Rows are sized here rather than measured, so the window fits them exactly. */
const HEADER_HEIGHT = 22;
const ROW_HEIGHT = 48;
const ROW_GAP = 8;
/** The shell's own padding plus the gap between the header and the first row. */
const CHROME = 16 + HEADER_HEIGHT + 8;

const SETTINGS_SECTION = "screen-recording";

function clock(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

/** The preset whose key is `key`, or `None` for a custom size. */
function presetOf(key: string) {
  return FIXED_SIZES.find((size) => size.key === key) ?? null;
}

export function RecordBar() {
  const { settings, patch } = useSettings();
  const [status, setStatus] = useState<RecordStatus>({ phase: "idle", elapsedMs: 0 });
  const [toolchain, setToolchain] = useState<RecordToolchain | null>(null);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** "1920x1080" / "1280x720" / "854x480" / "custom"; unrelated to the mode string. */
  const [sizeKey, setSizeKey] = useState<string>(FIXED_SIZES[0].key);
  const [customWidth, setCustomWidth] = useState("1280");
  const [customHeight, setCustomHeight] = useState("720");
  const mounted = useRef(true);

  const active = status.phase !== "idle";
  const audioOn = Boolean(settings?.recordAudioDevice);
  const custom = sizeKey === "custom";

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // The encoder and its devices: a fresh install has neither, which is what turns
  // the toolbar into the one thing that can be done about it.
  useEffect(() => {
    recordToolchain()
      .then((found) => {
        if (mounted.current) setToolchain(found);
      })
      .catch(() => undefined);
  }, []);

  // A bar opened while a recording runs starts on the controls, not the toolbar.
  useEffect(() => {
    let cancelled = false;
    const read = () => {
      recordStatus()
        .then((next) => {
          if (!cancelled && mounted.current) setStatus(next);
        })
        .catch(() => undefined);
    };
    read();
    const timer = window.setInterval(read, 500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  // Settings can arrive after the first paint, so the remembered size is adopted
  // once rather than on every change (which would fight the dropdown).
  const adopted = useRef(false);
  useEffect(() => {
    if (adopted.current || !settings) return;
    adopted.current = true;
    setCustomWidth(String(settings.recordFixedWidth));
    setCustomHeight(String(settings.recordFixedHeight));
    const mode = settings.recordMode;
    if (mode.startsWith("fixed:")) {
      const key = mode.slice("fixed:".length);
      setSizeKey(presetOf(key) ? key : "custom");
    }
  }, [settings]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<RecordStatus & { error?: string }>("record:state", (event) => {
      const { error: failure, ...next } = event.payload;
      if (!mounted.current) return;
      if (failure) setError(failure);
      // A failure leaves the session idle, so the toolbar comes back rather than
      // sitting there with controls for a recording that never started.
      setStatus(next);
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Saving happens on a worker thread, so the bar waits for the finished event
  // rather than for the stop call: the app window comes back once the clip exists.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<{ id?: number; error?: string }>("record:finished", (event) => {
      if (!mounted.current) return;
      setSaving(false);
      if (event.payload.error) {
        setError(event.payload.error);
        setStatus({ phase: "idle", elapsedMs: 0 });
        return;
      }
      void getCurrentWindow()
        .close()
        .catch(() => undefined);
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The window is sized to the rows it is showing, so it never covers more of the
  // screen than it needs to. The mode buttons, the size, the switches and the
  // header share one row; only the device list and the custom size get their own.
  const rows = (active ? 1 : 1 + (audioOn ? 1 : 0) + (custom ? 1 : 0)) + (error || notice ? 1 : 0);
  const height = CHROME + rows * ROW_HEIGHT + (rows - 1) * ROW_GAP + 4;
  useEffect(() => {
    getCurrentWindow()
      .setSize(new LogicalSize(WIDTH, height))
      .catch(() => undefined);
  }, [height]);

  const run = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  /** Starts a recording, remembering the mode so a shortcut repeats it. */
  const start = (mode: string, action: () => Promise<unknown>) => {
    void run(async () => {
      await patch({ recordMode: mode });
      await action();
    });
  };

  const startFixed = () => {
    const size = presetOf(sizeKey);
    const width = size?.width ?? Math.max(64, Number(customWidth) || 1280);
    const height = size?.height ?? Math.max(64, Number(customHeight) || 720);
    start(`fixed:${width}x${height}`, () => recordFixed(width, height));
  };

  /** Switches sound on by choosing a device, or off by clearing the choice. */
  const toggleAudio = () => {
    if (audioOn) {
      void patch({ recordAudioDevice: "" });
      return;
    }
    const first = toolchain?.devices[0];
    if (!first) {
      setError(
        "No recording devices were found. Windows has to have one enabled in Sound settings — a microphone, or a loopback device such as Stereo Mix for the machine's own sound.",
      );
      return;
    }
    setError(null);
    void patch({ recordAudioDevice: first });
  };

  if (toolchain && !toolchain.ready) {
    return (
      <Shell>
        <Header />
        <div className="flex flex-1 items-center gap-2">
          <p className="min-w-0 flex-1 text-[13px] text-ink-dim">
            Screen recording needs <span className="text-ink">FFmpeg</span>.
          </p>
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                setNotice("Installing FFmpeg…");
                await installFfmpeg();
                setToolchain(await recordToolchain());
                setNotice(null);
              })
            }
            className="shrink-0 rounded bg-accent/25 px-3 py-2 text-[13px] font-medium text-accent-hi transition-colors hover:bg-accent/40 disabled:opacity-50"
          >
            {busy ? "Installing…" : "Install"}
          </button>
        </div>
        {(error || notice) && <Failure>{error ?? notice}</Failure>}
      </Shell>
    );
  }

  if (active) {
    return (
      <Shell>
        <Header />
        <div className="flex flex-1 items-center gap-2">
          <span
            className={`size-3 shrink-0 rounded-full ${
              status.phase === "paused" ? "bg-ink-faint" : "animate-pulse bg-danger"
            }`}
            aria-hidden="true"
          />
          <span className="w-20 shrink-0 font-mono text-[20px] tabular-nums text-ink">
            {clock(status.elapsedMs)}
          </span>
          <span className="min-w-0 flex-1 truncate text-[13px] text-ink-faint">
            {status.phase === "paused" ? "Paused" : "Recording"}
          </span>
          <IconButton
            label={status.phase === "paused" ? "Resume recording" : "Pause recording"}
            onClick={() =>
              void run(() => (status.phase === "paused" ? recordResume() : recordPause()))
            }
          >
            {status.phase === "paused" ? (
              <IconPlay className="size-7" />
            ) : (
              <IconPause className="size-7" />
            )}
          </IconButton>
          <button
            type="button"
            disabled={busy || saving}
            onClick={() => {
              setSaving(true);
              setError(null);
              recordStop().catch((e) => {
                setSaving(false);
                setError(errorMessage(e));
              });
            }}
            className="flex shrink-0 items-center gap-2 rounded bg-danger/20 px-3 py-2 text-[13px] font-medium text-danger transition-colors hover:bg-danger/30 disabled:opacity-50"
          >
            <span className="size-3 rounded-sm bg-danger" aria-hidden="true" />
            {saving ? "Saving…" : "Stop"}
          </button>
        </div>
        {error && <Failure>{error}</Failure>}
      </Shell>
    );
  }

  const devices = toolchain?.devices ?? [];
  const audio = settings?.recordAudioDevice ?? "";

  return (
    <Shell>
      <Header />
      <div className="flex flex-1 flex-col gap-2">
        <div className="flex items-center gap-1">
          <ModeButton
            label="Record the whole screen"
            onClick={() => start("screen", recordScreen)}
            disabled={busy}
          >
            <IconFullScreenCapture className="size-7" />
          </ModeButton>
          <ModeButton
            label="Record a window"
            onClick={() => start("window", recordWindow)}
            disabled={busy}
          >
            <IconWindowCapture className="size-7" />
          </ModeButton>
          <ModeButton
            label="Record a region"
            onClick={() => start("region", recordRegion)}
            disabled={busy}
          >
            <IconRegionCapture className="size-7" />
          </ModeButton>
          <ModeButton
            label={
              custom
                ? `Record a custom ${customWidth} × ${customHeight} area`
                : `Record a fixed ${presetOf(sizeKey)?.label} area`
            }
            onClick={startFixed}
            disabled={busy}
          >
            <IconVideo className="size-7" />
          </ModeButton>
          <select
            aria-label="Fixed size"
            title="Size for the fixed-area recording"
            value={sizeKey}
            onChange={(event) => setSizeKey(event.target.value)}
            className="h-12 w-32 shrink-0 rounded border border-edge bg-panel-2 px-2 text-[13px] text-ink-dim focus:border-accent-dim focus:outline-none"
          >
            {FIXED_SIZES.map((size) => (
              <option key={size.key} value={size.key}>
                {size.label}
              </option>
            ))}
            <option value="custom">Custom…</option>
          </select>

          <span className="flex-1" />
          <span className="mx-1 h-6 w-px shrink-0 bg-edge" aria-hidden="true" />

          <Toggle
            label="Record sound"
            pressed={audioOn}
            onClick={toggleAudio}
            disabled={!toolchain}
          >
            <IconMicrophone className="size-7" />
          </Toggle>
          <Toggle
            label="Draw the pointer into the recording"
            pressed={settings?.recordMouse ?? true}
            onClick={() =>
              void patch({ recordMouse: !(settings?.recordMouse ?? true) })
            }
          >
            <IconPointer className="size-7" />
          </Toggle>
        </div>

        {audioOn && (
          <select
            aria-label="Audio source"
            title="What the recording listens to"
            value={audio}
            onChange={(event) => void patch({ recordAudioDevice: event.target.value })}
            className="h-12 w-full rounded border border-edge bg-panel-2 px-2 text-[13px] text-ink-dim focus:border-accent-dim focus:outline-none"
          >
            {devices.length === 0 && <option value="">No audio devices found</option>}
            {devices.map((device) => (
              <option key={device} value={device}>
                {device}
              </option>
            ))}
            {/* A device that has since been unplugged stays selectable, so the
                setting is not silently lost when the recorder cannot see it. */}
            {audio && !devices.includes(audio) && <option value={audio}>{audio}</option>}
          </select>
        )}

        {custom && (
          <div className="flex items-center gap-2">
            <input
              type="number"
              aria-label="Custom width"
              title="Width in pixels"
              value={customWidth}
              onChange={(event) => setCustomWidth(event.target.value)}
              onBlur={() => void patch({ recordFixedWidth: Number(customWidth) || 1280 })}
              min={64}
              max={7680}
              className="h-12 w-24 rounded border border-edge bg-panel-2 px-2 text-[13px] text-ink focus:border-accent-dim focus:outline-none"
            />
            <span className="text-[12px] text-ink-faint">×</span>
            <input
              type="number"
              aria-label="Custom height"
              title="Height in pixels"
              value={customHeight}
              onChange={(event) => setCustomHeight(event.target.value)}
              onBlur={() => void patch({ recordFixedHeight: Number(customHeight) || 720 })}
              min={64}
              max={4320}
              className="h-12 w-24 rounded border border-edge bg-panel-2 px-2 text-[13px] text-ink focus:border-accent-dim focus:outline-none"
            />
            <span className="min-w-0 flex-1 truncate text-[11px] text-ink-faint">
              pixels, placed by dragging
            </span>
          </div>
        )}

        {(error || notice) && <Failure>{error ?? notice}</Failure>}
      </div>
    </Shell>
  );
}

/** The drag strip: window commands on the right, empty room for the pointer. */
function Header() {
  return (
    <div
      data-drag-window
      className="flex h-[22px] shrink-0 cursor-grab items-center gap-1 active:cursor-grabbing"
    >
      <span data-drag-window className="h-full min-w-0 flex-1" />
      <IconButton
        label="Screen recording settings"
        title="Screen recording settings"
        onClick={() => void openSettingsSection(SETTINGS_SECTION)}
      >
        <IconSettings className="size-3.5" />
      </IconButton>
      <IconButton
        label="Close the recording bar"
        onClick={() => void getCurrentWindow().close().catch(() => undefined)}
      >
        <IconClose className="size-3" />
      </IconButton>
    </div>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full w-full flex-col gap-2 rounded-panel border border-edge bg-panel/95 px-3 py-2 shadow-xl backdrop-blur">
      {children}
    </div>
  );
}

function Failure({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[11px] leading-snug text-danger" role="alert">
      {children}
    </p>
  );
}

function ModeButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className="flex size-12 shrink-0 items-center justify-center rounded border border-edge bg-panel-2 text-ink-faint transition-colors hover:border-accent-dim hover:text-ink disabled:opacity-50"
    >
      {children}
    </button>
  );
}

/** A switch that stays pressed, rather than a checkbox: it reads at a glance. */
function Toggle({
  label,
  pressed,
  disabled,
  onClick,
  children,
}: {
  label: string;
  pressed: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={pressed}
      aria-label={label}
      title={label}
      className={`flex size-12 shrink-0 items-center justify-center rounded border transition-colors disabled:opacity-50 ${
        pressed
          ? "border-accent-dim bg-accent/20 text-accent-hi"
          : "border-edge bg-panel-2 text-ink-faint hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

function IconButton({
  label,
  title,
  onClick,
  children,
}: {
  label: string;
  title?: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title ?? label}
      aria-label={label}
      className="flex size-5 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
    >
      {children}
    </button>
  );
}
