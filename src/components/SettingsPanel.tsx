import {
  Children,
  cloneElement,
  isValidElement,
  useEffect,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { formatBytes, formatCount, formatRelative } from "../lib/format";
import { errorMessage, storageInfo, validateHotkey, setStorageDir } from "../lib/ipc";
import { COLOR_FIELDS, resolveColors } from "../lib/theme";
import type { Settings, Stats, StorageInfo, Theme, ThemeMode, ViewMode } from "../lib/types";
import type { UpdateState } from "../hooks/useUpdater";
import { IconClose, IconSearch, IconShield } from "./icons";

interface SettingsPanelProps {
  settings: Settings | null;
  stats: Stats | null;
  error: string | null;
  notice: string | null;
  appVersion: string | null;
  updateState: UpdateState;
  onCheckForUpdates: () => void;
  onPatch: (delta: Partial<Settings>) => Promise<unknown>;
  onDismissError: () => void;
  onRevealDataDir: () => void;
  onClearHistory: (keepPinned: boolean) => void;
  onPrune: () => void;
}

const MEGABYTE = 1024 * 1024;

type RowProps = { label?: string; hint?: string };
type SectionProps = { title: string; hint?: string; children?: ReactNode };

function isRow(node: ReactNode): node is ReactElement<RowProps> {
  return isValidElement(node) && typeof (node.props as RowProps).label === "string";
}

function isSection(node: ReactNode): node is ReactElement<SectionProps> {
  return isValidElement(node) && typeof (node.props as SectionProps).title === "string";
}

/** Scores one term: 3 for a prefix hit, 2 for a hit further in, 0 for a miss. */
function termScore(text: string, term: string): number {
  const index = text.indexOf(term);
  if (index === 0) return 3;
  return index > 0 ? 2 : 0;
}

/**
 * Scores a name and description against every search term. Names weigh more than
 * descriptions, and every term has to land in one of the two.
 */
function textScore(label: string, hint: string | undefined, terms: string[]): number {
  let score = 0;
  for (const term of terms) {
    const nameHit = termScore(label.toLowerCase(), term);
    if (nameHit > 0) {
      score += nameHit;
      continue;
    }
    if (termScore((hint ?? "").toLowerCase(), term) > 0) {
      score += 1;
      continue;
    }
    return 0;
  }
  return score;
}

/**
 * Narrows the settings page to matching rows. A section whose own heading matches
 * keeps all of its rows, and matching rows sort above rows matched only on their
 * description.
 */
function filterSections(children: ReactNode, query: string): ReactElement[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const kept: ReactElement[] = [];

  for (const section of Children.toArray(children).filter(isSection)) {
    const nodes = Children.toArray(section.props.children);

    // No query: pass the section through untouched, so structural content that is
    // not a plain setting row (composite editors) stays visible.
    if (terms.length === 0) {
      if (nodes.length > 0) kept.push(cloneElement(section, undefined, nodes));
      continue;
    }

    const sectionHit = textScore(section.props.title, section.props.hint, terms) > 0;

    const rows = nodes
      .filter(isRow)
      .map((row, index) => ({
        row,
        index,
        score: textScore(row.props.label ?? "", row.props.hint, terms),
      }))
      .filter(({ score }) => score > 0 || sectionHit)
      .sort((a, b) => b.score - a.score || a.index - b.index)
      .map(({ row }) => row);

    if (rows.length > 0) kept.push(cloneElement(section, undefined, rows));
  }

  return kept;
}

function SettingsSections({ query, children }: { query: string; children: ReactNode }) {
  const sections = filterSections(children, query);

  if (sections.length === 0) {
    return (
      <p className="px-4 py-8 text-center text-[11px] text-ink-faint">
        No setting matches “{query.trim()}”.
      </p>
    );
  }

  return <>{sections}</>;
}

function SettingsSearch({ value, onChange }: { value: string; onChange: (next: string) => void }) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-edge bg-panel px-4 py-2">
      <IconSearch className="size-3.5 shrink-0 text-ink-faint" />

      <input
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          // Escape clears the filter before it falls through to closing settings.
          if (event.key === "Escape" && value) {
            event.preventDefault();
            event.stopPropagation();
            onChange("");
          }
        }}
        // The window is summoned to be typed into, so settings should open with
        // the caret already in the filter.
        autoFocus
        spellCheck={false}
        autoComplete="off"
        placeholder="Search settings…"
        aria-label="Search settings"
        className="min-w-0 flex-1 bg-transparent text-[13px] text-ink placeholder:text-ink-faint focus:outline-none"
      />

      {value && (
        <button
          type="button"
          onClick={() => onChange("")}
          title="Clear the settings search"
          aria-label="Clear the settings search"
          className="flex size-5 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
        >
          <IconClose className="size-3" />
        </button>
      )}
    </div>
  );
}

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
      className={`flex h-[18px] w-[32px] items-center overflow-hidden rounded-full px-[2px] transition-colors ${
        checked ? "bg-accent" : "bg-panel-3"
      }`}
    >
      {/* Flex centring plus a translate keeps the knob inside the pill at any
          interface scale; absolute pixel offsets rounded badly when zoomed. */}
      <span
        className={`size-[14px] shrink-0 rounded-full bg-white transition-transform ${
          checked ? "translate-x-[14px]" : "translate-x-0"
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

/** Segmented list/cards control used for the per-category view preferences. */
function ViewModeField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: ViewMode;
  onChange: (next: ViewMode) => void;
}) {
  return (
    <div
      className="flex items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5"
      role="group"
      aria-label={label}
    >
      {(["list", "grid"] as ViewMode[]).map((mode) => (
        <button
          key={mode}
          type="button"
          onClick={() => onChange(mode)}
          aria-pressed={value === mode}
          className={`rounded px-2 py-0.5 text-[10px] transition-colors ${
            value === mode
              ? "bg-accent/25 text-accent-hi"
              : "text-ink-faint hover:text-ink-dim"
          }`}
        >
          {mode === "list" ? "List" : "Cards"}
        </button>
      ))}
    </div>
  );
}

/** Colour swatch plus a hex field, so a value can be picked or typed. */
function ColorField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  const commit = () => {
    const candidate = draft.trim().startsWith("#") ? draft.trim() : `#${draft.trim()}`;
    if (/^#[0-9a-f]{6}$/i.test(candidate)) {
      onChange(candidate.toLowerCase());
    } else {
      setDraft(value);
    }
  };

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="color"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-label={`${label} colour picker`}
        className="size-6 cursor-pointer rounded border border-edge bg-panel-2 p-0"
      />
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
        aria-label={`${label} hex value`}
        className="w-[74px] rounded border border-edge bg-panel-2 px-1.5 py-1 text-center font-mono text-[10px] text-ink focus:border-accent-dim focus:outline-none"
      />
    </div>
  );
}

/**
 * Theme controls as individual rows, so the settings search can filter them like
 * any other setting.
 */
function themeRows(theme: Theme, onChange: (next: Theme) => void): ReactNode {
  const colors = resolveColors(theme);
  const modes: { id: ThemeMode; label: string }[] = [
    { id: "dark", label: "Dark" },
    { id: "light", label: "Light" },
    { id: "custom", label: "Custom" },
  ];

  const rows: ReactNode[] = [
    <Row
      key="theme-mode"
      label="Theme"
      hint="Presets repaint the whole interface; Custom exposes every colour."
      control={
        <div className="flex items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
          {modes.map((mode) => (
            <button
              key={mode.id}
              type="button"
              aria-pressed={theme.mode === mode.id}
              onClick={() =>
                // Carry the palette the user is looking at into custom mode so the
                // pickers start from the visible colours.
                onChange({ mode: mode.id, colors: resolveColors(theme) })
              }
              className={`rounded px-2 py-0.5 text-[10px] transition-colors ${
                theme.mode === mode.id
                  ? "bg-accent/25 text-accent-hi"
                  : "text-ink-faint hover:text-ink-dim"
              }`}
            >
              {mode.label}
            </button>
          ))}
        </div>
      }
    />,
  ];

  if (theme.mode === "custom") {
    rows.push(
      ...COLOR_FIELDS.map((field) => (
        <Row
          key={field.key}
          label={field.label}
          hint={field.hint}
          control={
            <ColorField
              label={field.label}
              value={colors[field.key]}
              onChange={(value) =>
                onChange({ mode: "custom", colors: { ...colors, [field.key]: value } })
              }
            />
          }
        />
      )),
    );
  }

  return rows;
}

/** Explains the update state in one line under the version row. */
function updateHint(settings: Settings, state: UpdateState, version: string | null): string {
  if (state.kind === "checking") return "Checking GitHub for a newer release…";
  if (state.kind === "available") return `Version ${state.version} is ready to install.`;
  if (state.kind === "downloading") return `Installing ${state.version}…`;
  if (state.kind === "error") return state.message;

  const name = version ? `Version ${version}` : "CopyPaste";
  return settings.lastUpdateCheck > 0
    ? `${name} · last checked ${formatRelative(settings.lastUpdateCheck)}`
    : `${name} · not checked yet`;
}

export function SettingsPanel({
  settings,
  stats,
  error,
  notice,
  appVersion,
  updateState,
  onCheckForUpdates,
  onPatch,
  onDismissError,
  onRevealDataDir,
  onClearHistory,
  onPrune,
}: SettingsPanelProps) {
  // Clearing everything is irreversible, so it takes two deliberate clicks.
  const [armed, setArmed] = useState(false);
  const [query, setQuery] = useState("");
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [pendingMove, setPendingMove] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [storageError, setStorageError] = useState<string | null>(null);
  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), 5_000);
    return () => window.clearTimeout(timer);
  }, [armed]);

  useEffect(() => {
    storageInfo()
      .then(setStorage)
      .catch(() => undefined);
  }, []);

  const chooseStorage = async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose where CopyPaste stores its clips",
        defaultPath: storage?.dataDir,
      });
      if (typeof picked === "string") {
        setStorageError(null);
        setPendingMove(picked);
      }
    } catch (e) {
      setStorageError(errorMessage(e));
    }
  };

  const runMove = async (target: string) => {
    setMoving(true);
    setStorageError(null);
    try {
      // The backend copies the data and restarts, so this normally never returns.
      await setStorageDir(target);
    } catch (e) {
      setStorageError(errorMessage(e));
      setMoving(false);
      setPendingMove(null);
    }
  };

  if (!settings) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <p className="text-[12px] text-ink-faint">Loading settings…</p>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SettingsSearch value={query} onChange={setQuery} />

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

        <SettingsSections query={query}>
          <Section
            title="Appearance"
            hint="Layout and sizes apply immediately and are remembered."
          >
            <Row
              label="Show in the taskbar"
              hint="Off leaves the app in the tray only."
              control={
                <Toggle
                  label="Show in the taskbar"
                  checked={settings.showInTaskbar}
                  onChange={(next) => void onPatch({ showInTaskbar: next })}
                />
              }
            />
            <Row
              label="All clips"
              hint="Also switchable from the toolbar, which remembers the choice."
              control={
                <ViewModeField
                  label="All clips layout"
                  value={settings.viewModes.all}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, all: mode } })
                  }
                />
              }
            />
            <Row
              label="Text"
              hint="Rows suit reading text."
              control={
                <ViewModeField
                  label="Text layout"
                  value={settings.viewModes.text}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, text: mode } })
                  }
                />
              }
            />
            <Row
              label="Images"
              hint="Cards are the default here."
              control={
                <ViewModeField
                  label="Images layout"
                  value={settings.viewModes.images}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, images: mode } })
                  }
                />
              }
            />
            <Row
              label="Files"
              control={
                <ViewModeField
                  label="Files layout"
                  value={settings.viewModes.files}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, files: mode } })
                  }
                />
              }
            />
            <Row
              label="Pinned"
              control={
                <ViewModeField
                  label="Pinned layout"
                  value={settings.viewModes.pinned}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, pinned: mode } })
                  }
                />
              }
            />
            <Row
              label="Collections and tags"
              hint="Used for every collection, archive and tag view."
              control={
                <ViewModeField
                  label="Collections and tags layout"
                  value={settings.viewModes.other}
                  onChange={(mode) =>
                    void onPatch({ viewModes: { ...settings.viewModes, other: mode } })
                  }
                />
              }
            />
            <Row
              label="Card size"
              hint="Also adjustable with the slider in the toolbar."
              control={
                <NumberField
                  value={Math.round(settings.gridScale * 100)}
                  min={60}
                  max={200}
                  suffix="%"
                  onChange={(next) => void onPatch({ gridScale: next / 100 })}
                />
              }
            />
            <Row
              label="Text scale"
              hint="Scales the whole interface, previews included."
              control={
                <NumberField
                  value={settings.textScale}
                  min={70}
                  max={200}
                  suffix="%"
                  onChange={(next) => void onPatch({ textScale: next })}
                />
              }
            />
            <Row
              label="Preview pane"
              hint="Also toggled by the chevron at the top of the pane."
              control={
                <Toggle
                  label="Show the preview pane"
                  checked={settings.detailPaneOpen}
                  onChange={(next) => void onPatch({ detailPaneOpen: next })}
                />
              }
            />
            {themeRows(settings.theme, (next) => void onPatch({ theme: next }))}
          </Section>

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
              hint="0 keeps everything. Choose hours or days; archived clips are exempt."
              control={
                <div className="flex items-center gap-1.5">
                  <NumberField
                    value={settings.maxAgeHours > 0 ? settings.maxAgeHours : settings.maxAgeDays}
                    min={0}
                    max={settings.maxAgeHours > 0 ? 87_600 : 3_650}
                    suffix={settings.maxAgeHours > 0 ? "hours" : "days"}
                    onChange={(next) =>
                      void onPatch(
                        settings.maxAgeHours > 0
                          ? { maxAgeHours: next, maxAgeDays: 0 }
                          : { maxAgeDays: next, maxAgeHours: 0 },
                      )
                    }
                  />
                  <div className="flex items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
                    <button
                      type="button"
                      aria-pressed={settings.maxAgeHours > 0}
                      onClick={() =>
                        void onPatch({
                          maxAgeHours: settings.maxAgeDays * 24,
                          maxAgeDays: 0,
                        })
                      }
                      className={`rounded px-1.5 py-0.5 text-[10px] transition-colors ${
                        settings.maxAgeHours > 0
                          ? "bg-accent/25 text-accent-hi"
                          : "text-ink-faint hover:text-ink-dim"
                      }`}
                    >
                      Hours
                    </button>
                    <button
                      type="button"
                      aria-pressed={settings.maxAgeHours === 0}
                      onClick={() =>
                        void onPatch({
                          maxAgeDays:
                            settings.maxAgeHours > 0
                              ? Math.max(1, Math.round(settings.maxAgeHours / 24))
                              : 0,
                          maxAgeHours: 0,
                        })
                      }
                      className={`rounded px-1.5 py-0.5 text-[10px] transition-colors ${
                        settings.maxAgeHours === 0
                          ? "bg-accent/25 text-accent-hi"
                          : "text-ink-faint hover:text-ink-dim"
                      }`}
                    >
                      Days
                    </button>
                  </div>
                </div>
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

          <Section
            title="Updates"
            hint="Signed installers come from the project's GitHub releases."
          >
            <Row
              label="Automatic updates"
              hint="Checks once a day; nothing is downloaded until a release matches."
              control={
                <Toggle
                  label="Automatic updates"
                  checked={settings.autoUpdate}
                  onChange={(next) => void onPatch({ autoUpdate: next })}
                />
              }
            />
            <Row
              label="Version"
              hint={updateHint(settings, updateState, appVersion)}
              control={
                <button
                  type="button"
                  onClick={onCheckForUpdates}
                  disabled={updateState.kind === "checking" || updateState.kind === "downloading"}
                  className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                >
                  {updateState.kind === "checking" ? "Checking…" : "Check now"}
                </button>
              }
            />
          </Section>

          <Section
            title="Storage"
            hint="Moving the folder copies the database, images and key, then restarts CopyPaste."
          >
            <Row
              label="Data folder"
              hint={
                storage
                  ? storage.dataDir
                  : stats
                    ? `${formatBytes(stats.databaseBytes)} database · ${formatBytes(stats.mediaBytes)} media`
                    : undefined
              }
              control={
                <div className="flex gap-1.5">
                  <button
                    type="button"
                    onClick={onRevealDataDir}
                    className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                  >
                    Open
                  </button>
                  <button
                    type="button"
                    onClick={() => void chooseStorage()}
                    disabled={moving}
                    className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                  >
                    Change…
                  </button>
                  {storage?.custom && (
                    <button
                      type="button"
                      onClick={() => setPendingMove(storage.defaultDir)}
                      disabled={moving}
                      title={storage.defaultDir}
                      className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                    >
                      Reset
                    </button>
                  )}
                </div>
              }
            />

            {pendingMove && (
              <Row
                label={
                  storage?.custom && pendingMove === storage.defaultDir
                    ? "Move back to the default folder?"
                    : "Move the clips to this folder?"
                }
                hint={moving ? "Copying clips and restarting…" : pendingMove}
                control={
                  <div className="flex gap-1.5">
                    <button
                      type="button"
                      onClick={() => setPendingMove(null)}
                      disabled={moving}
                      className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={() => void runMove(pendingMove)}
                      disabled={moving}
                      className="rounded border border-accent-dim bg-accent/20 px-2.5 py-1 text-[11px] text-accent-hi transition-colors hover:bg-accent/30 disabled:opacity-40"
                    >
                      Move &amp; restart
                    </button>
                  </div>
                }
              />
            )}

            {storageError && (
              <p className="py-2 text-[10px] leading-relaxed text-danger">{storageError}</p>
            )}

            <Row
              label="Clear history"
              hint={
                stats
                  ? `${formatCount(stats.total, "clip")} stored, ${stats.pinned} pinned${
                      stats.archived > 0 ? `, ${stats.archived} archived` : ""
                    }. Archived clips are never cleared here.`
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
        </SettingsSections>

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
    </div>
  );
}
