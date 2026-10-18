import { useEffect, useState, type ReactNode } from "react";

import { formatBytes, formatCount } from "../lib/format";
import { errorMessage, validateHotkey } from "../lib/ipc";
import type { Settings, Stats } from "../lib/types";
import { IconShield } from "./icons";

interface SettingsPanelProps {
  settings: Settings | null;
  stats: Stats | null;
  error: string | null;
  notice: string | null;
  onPatch: (delta: Partial<Settings>) => Promise<unknown>;
  onDismissError: () => void;
  onRevealDataDir: () => void;
  onClearHistory: (keepPinned: boolean) => void;
  onPrune: () => void;
}

const MEGABYTE = 1024 * 1024;

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="border-t border-edge-soft px-4 py-3 first:border-t-0">
      <h3 className="text-[11px] font-semibold text-ink">{title}</h3>
      {hint && <p className="mt-0.5 text-[10px] leading-relaxed text-ink-faint">{hint}</p>}
      <div className="mt-2 divide-y divide-edge-soft">{children}</div>
    </section>
  );
}

function Row({
  label,
  hint,
  control,
}: {
  label: string;
  hint?: string;
  control: ReactNode;
}) {
  return (
    <div className="flex items-start gap-4 py-2">
      <div className="min-w-0 flex-1">
        <p className="text-[12px] text-ink-dim">{label}</p>
        {hint && <p className="mt-0.5 text-[10px] leading-relaxed text-ink-faint">{hint}</p>}
      </div>
      <div className="shrink-0 pt-px">{control}</div>
    </div>
  );
}

function Toggle({
  checked,
  label,
  onChange,
}: {
  checked: boolean;
  label: string;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-[18px] w-[32px] rounded-full transition-colors ${
        checked ? "bg-accent" : "bg-panel-3"
      }`}
    >
      <span
        className={`absolute top-[2px] size-[14px] rounded-full bg-white transition-transform ${
          checked ? "translate-x-[16px]" : "translate-x-[2px]"
        }`}
      />
    </button>
  );
}

function NumberField({
  value,
  min,
  max,
  suffix,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  suffix?: string;
  onChange: (next: number) => void;
}) {
  // Kept in local state so intermediate values ("12" while typing "120") survive;
  // the backend clamps whatever it finally receives.
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="number"
        value={draft}
        min={min}
        max={max}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => {
          const parsed = Number(draft);
          if (Number.isFinite(parsed)) {
            onChange(Math.min(max, Math.max(min, Math.round(parsed))));
          } else {
            setDraft(String(value));
          }
        }}
        className="w-20 rounded border border-edge bg-panel-2 px-2 py-1 text-right text-[11px] tabular-nums text-ink focus:border-accent-dim focus:outline-none"
      />
      {suffix && <span className="text-[10px] text-ink-faint">{suffix}</span>}
    </div>
  );
}

function MegabyteField({
  bytes,
  onChange,
}: {
  bytes: number;
  onChange: (nextBytes: number) => void;
}) {
  return (
    <NumberField
      value={Math.round(bytes / MEGABYTE)}
      min={1}
      max={512}
      suffix="MB"
      onChange={(megabytes) => onChange(megabytes * MEGABYTE)}
    />
  );
}

function HotkeyField({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (next: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => setDraft(value), [value]);

  // Validate as the user types so a conflict or typo is visible before committing.
  useEffect(() => {
    if (draft.trim() === value) {
      setProblem(null);
      return;
    }
    const timer = window.setTimeout(() => {
      validateHotkey(draft.trim())
        .then(() => setProblem(null))
        .catch((e) => setProblem(errorMessage(e)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [draft, value]);

  const commit = () => {
    const next = draft.trim();
    if (!next || next === value) {
      setDraft(value);
      return;
    }
    onCommit(next);
  };

  return (
    <div className="text-right">
      <input
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(value);
          event.stopPropagation();
        }}
        spellCheck={false}
        aria-label="Global hotkey"
        aria-invalid={problem !== null}
        className={`w-36 rounded border bg-panel-2 px-2 py-1 text-center font-mono text-[11px] text-ink focus:outline-none ${
          problem ? "border-danger" : "border-edge focus:border-accent-dim"
        }`}
      />
      {problem && <p className="mt-1 max-w-[16rem] text-[10px] text-danger">{problem}</p>}
    </div>
  );
}

export function SettingsPanel({
  settings,
  stats,
  error,
  notice,
  onPatch,
  onDismissError,
  onRevealDataDir,
  onClearHistory,
  onPrune,
}: SettingsPanelProps) {
  // Clearing everything is irreversible, so it takes two deliberate clicks.
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), 5_000);
    return () => window.clearTimeout(timer);
  }, [armed]);

  if (!settings) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <p className="text-[12px] text-ink-faint">Loading settings…</p>
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      {(error || notice) && (
        <div
          role="status"
          className={`flex items-start gap-2 border-b px-4 py-2 text-[11px] ${
            error
              ? "border-danger/30 bg-danger/10 text-danger"
              : "border-ok/30 bg-ok/10 text-ok"
          }`}
        >
          <span className="min-w-0 flex-1">{error ?? notice}</span>
          {error && (
            <button
              type="button"
              onClick={onDismissError}
              className="shrink-0 underline underline-offset-2"
            >
              Dismiss
            </button>
          )}
        </div>
      )}

      <Section
        title="Activation"
        hint="The shortcut that summons this window from anywhere. It must be unique across the system."
      >
        <Row
          label="Global hotkey"
          hint="Combinations are written with `+`, for example Alt+V or Ctrl+Shift+V."
          control={<HotkeyField value={settings.hotkey} onCommit={(next) => void onPatch({ hotkey: next })} />}
        />
        <Row
          label="Launch at login"
          hint="Start CopyPaste minimised to the tray when you sign in."
          control={
            <Toggle
              label="Launch at login"
              checked={settings.launchAtLogin}
              onChange={(next) => void onPatch({ launchAtLogin: next })}
            />
          }
        />
      </Section>

      <Section
        title="Pasting"
        hint="Auto-paste sends Ctrl+V to the window you were in before opening CopyPaste."
      >
        <Row
          label="Paste automatically"
          hint="Off by default: the clip is placed on the clipboard and you press Ctrl+V yourself."
          control={
            <Toggle
              label="Paste automatically"
              checked={settings.autoPaste}
              onChange={(next) => void onPatch({ autoPaste: next })}
            />
          }
        />
        <Row
          label="Delay before pasting"
          hint="Longer values help if the previous window is slow to take focus back."
          control={
            <NumberField
              value={settings.pasteDelayMs}
              min={0}
              max={2000}
              suffix="ms"
              onChange={(next) => void onPatch({ pasteDelayMs: next })}
            />
          }
        />
      </Section>

      <Section title="Window">
        <Row
          label="Hide when focus is lost"
          control={
            <Toggle
              label="Hide when focus is lost"
              checked={settings.hideOnBlur}
              onChange={(next) => void onPatch({ hideOnBlur: next })}
            />
          }
        />
        <Row
          label="Start hidden"
          hint="When off, the window opens on every launch instead of waiting for the hotkey."
          control={
            <Toggle
              label="Start hidden"
              checked={settings.startHidden}
              onChange={(next) => void onPatch({ startHidden: next })}
            />
          }
        />
      </Section>

      <Section title="Capture">
        <Row
          label="Capture images"
          control={
            <Toggle
              label="Capture images"
              checked={settings.captureImages}
              onChange={(next) => void onPatch({ captureImages: next })}
            />
          }
        />
        <Row
          label="Capture file lists"
          hint="Records the paths when you copy files in File Explorer."
          control={
            <Toggle
              label="Capture file lists"
              checked={settings.captureFiles}
              onChange={(next) => void onPatch({ captureFiles: next })}
            />
          }
        />
        <Row
          label="Prefer images over text"
          hint="Some apps offer both a bitmap and its URL. Off means the text wins."
          control={
            <Toggle
              label="Prefer images over text"
              checked={settings.preferImagesOverText}
              onChange={(next) => void onPatch({ preferImagesOverText: next })}
            />
          }
        />
        <Row
          label="Respect app opt-outs"
          hint="Skips anything a password manager has flagged as excluded from history. Leave this on."
          control={
            <Toggle
              label="Respect app opt-outs"
              checked={settings.ignoreSensitive}
              onChange={(next) => void onPatch({ ignoreSensitive: next })}
            />
          }
        />
        <Row
          label="Ignored applications"
          hint="One process name per line. Copies made in these apps are never recorded."
          control={
            <textarea
              value={settings.ignoredApps.join("\n")}
              onChange={(event) =>
                void onPatch({
                  ignoredApps: event.target.value
                    .split("\n")
                    .map((line) => line.trim())
                    .filter(Boolean),
                })
              }
              onKeyDown={(event) => event.stopPropagation()}
              rows={4}
              spellCheck={false}
              aria-label="Ignored applications"
              placeholder={"keepassxc.exe\nbitwarden.exe"}
              className="w-44 resize-none rounded border border-edge bg-panel-2 px-2 py-1 font-mono text-[10px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
            />
          }
        />
      </Section>

      <Section
        title="Retention"
        hint="Pinned clips are never removed, and they do not count toward the limit."
      >
        <Row
          label="Maximum clips"
          hint="0 keeps everything."
          control={
            <NumberField
              value={settings.maxItems}
              min={0}
              max={100000}
              onChange={(next) => void onPatch({ maxItems: next })}
            />
          }
        />
        <Row
          label="Maximum age"
          hint="0 keeps everything. Older unpinned clips are dropped on the next copy."
          control={
            <NumberField
              value={settings.maxAgeDays}
              min={0}
              max={3650}
              suffix="days"
              onChange={(next) => void onPatch({ maxAgeDays: next })}
            />
          }
        />
        <Row
          label="Apply retention now"
          control={
            <button
              type="button"
              onClick={onPrune}
              className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
            >
              Prune
            </button>
          }
        />
      </Section>

      <Section title="Limits">
        <Row
          label="Largest text clip"
          hint="Anything bigger is ignored rather than stored."
          control={
            <MegabyteField
              bytes={settings.maxTextBytes}
              onChange={(bytes) => void onPatch({ maxTextBytes: bytes })}
            />
          }
        />
        <Row
          label="Largest image clip"
          control={
            <MegabyteField
              bytes={settings.maxImageBytes}
              onChange={(bytes) => void onPatch({ maxImageBytes: bytes })}
            />
          }
        />
        <Row
          label="Clipboard poll interval"
          hint="How often the clipboard is checked for changes."
          control={
            <NumberField
              value={settings.pollIntervalMs}
              min={120}
              max={5000}
              suffix="ms"
              onChange={(next) => void onPatch({ pollIntervalMs: next })}
            />
          }
        />
      </Section>

      <Section title="Storage">
        <Row
          label="Data folder"
          hint={stats ? `${formatBytes(stats.databaseBytes)} database · ${formatBytes(stats.mediaBytes)} media` : undefined}
          control={
            <button
              type="button"
              onClick={onRevealDataDir}
              className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
            >
              Open folder
            </button>
          }
        />
        <Row
          label="Clear history"
          hint={
            stats
              ? `${formatCount(stats.total, "clip")} stored, ${stats.pinned} pinned`
              : undefined
          }
          control={
            <div className="flex gap-1.5">
              <button
                type="button"
                onClick={() => onClearHistory(true)}
                className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
              >
                Unpinned
              </button>
              <button
                type="button"
                onClick={() => {
                  if (!armed) {
                    setArmed(true);
                    return;
                  }
                  setArmed(false);
                  onClearHistory(false);
                }}
                className={`rounded border px-2.5 py-1 text-[11px] transition-colors ${
                  armed
                    ? "border-danger bg-danger text-white hover:bg-danger/90"
                    : "border-danger/40 bg-danger/10 text-danger hover:bg-danger/20"
                }`}
              >
                {armed ? "Confirm — delete all" : "Everything"}
              </button>
            </div>
          }
        />
      </Section>

      <section className="flex items-start gap-2 border-t border-edge-soft px-4 py-3">
        <IconShield className="mt-px size-3.5 shrink-0 text-ok" />
        <p className="text-[10px] leading-relaxed text-ink-faint">
          {stats?.encryption ?? "Encrypted local storage"}. Clip contents and previews are
          encrypted individually; the key is sealed to your Windows account, so the files in
          this folder are not readable by another user or on another machine. Nothing is ever
          sent anywhere.
        </p>
      </section>
    </div>
  );
}
