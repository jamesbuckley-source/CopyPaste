import {
  Children,
  Fragment,
  cloneElement,
  isValidElement,
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { formatBytes, formatCount, formatRelative } from "../lib/format";
import { errorMessage, copyText, storageInfo, validateHotkey, setStorageDir } from "../lib/ipc";
import { COLOR_FIELDS, resolveColors } from "../lib/theme";
import {
  DEFAULT_CAPTURE_OUTLINE,
  type Settings,
  type Stats,
  type StorageInfo,
  type VaultStatus,
  type Theme,
  type ThemeMode,
  type ViewMode,
} from "../lib/types";
import type { UpdateState } from "../hooks/useUpdater";
import { CaptureModeField } from "./CaptureModeField";
import { IconChevronRight, IconClose, IconSearch } from "./icons";

interface SettingsPanelProps {
  settings: Settings | null;
  stats: Stats | null;
  error: string | null;
  notice: string | null;
  appVersion: string | null;
  updateState: UpdateState;
  onCheckForUpdates: () => void;
  /** Reopens the first-run setup wizard. */
  onRunSetup: () => void;
  /** Opens the GitHub release notes for a version in the browser. */
  onOpenChangelog: (version: string) => void;
  /** Opens the project's GitHub page in the browser. */
  onOpenProject: () => void;
  /** Asks for a WAV to play for a copy outcome. */
  onChooseSound: (kind: "copy" | "fail") => void;
  onPatch: (delta: Partial<Settings>) => Promise<unknown>;
  /** Whether sealed clips can be read, and the three things that can change it. */
  vaultStatus: VaultStatus;
  onVaultPrompt: (kind: "create" | "unlock" | "change") => void;
  onLockVault: () => void;
  onDismissError: () => void;
  onRevealDataDir: () => void;
  onClearHistory: (keepPinned: boolean) => void;
  onPrune: () => void;
  /** Unix-millisecond deadline of a timed pause, or null. */
  pausedUntil: number | null;
  pausedIndefinitely: boolean;
  onPause: (minutes?: number, untilRestart?: boolean) => void;
  onResume: () => void;
  now: number;
}

const MEGABYTE = 1024 * 1024;

type RowProps = { label?: string; hint?: string };
type SectionProps = { title: string; hint?: string; children?: ReactNode };

/**
 * Expansion state for the collapsible sections. Context rather than props because
 * the search filter re-creates the sections, and cloning should not have to know
 * about it.
 */
type SectionStateValue = {
  forcedOpen: boolean;
  isOpen: (title: string) => boolean;
  toggle: (title: string) => void;
};

const SectionState = createContext<SectionStateValue | null>(null);

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

function SettingsSections({
  query,
  sectionState,
  children,
}: {
  query: string;
  sectionState: SectionStateValue;
  children: ReactNode;
}) {
  const sections = filterSections(children, query);

  if (sections.length === 0) {
    return (
      <p className="px-4 py-8 text-center text-[11px] text-ink-faint">
        No setting matches “{query.trim()}”.
      </p>
    );
  }

  return <SectionState.Provider value={sectionState}>{sections}</SectionState.Provider>;
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
  const state = useContext(SectionState);
  // A search result has to be readable, so a matching section shows its rows
  // whatever the collapsed set says. Without a provider the section is static.
  const forced = state === null || state.forcedOpen;
  const open = forced || state.isOpen(title);

  const heading = (
    <>
      {!forced && (
        <IconChevronRight
          className={`mt-0.5 size-3 shrink-0 text-ink-faint transition-transform duration-150 ${
            open ? "rotate-90" : ""
          }`}
        />
      )}
      <span className="min-w-0 flex-1">
        <span className="block text-[11px] font-semibold text-ink">{title}</span>
        {hint && <span className="mt-0.5 block text-[10px] leading-relaxed text-ink-faint">{hint}</span>}
      </span>
    </>
  );

  return (
    <section className="border-t border-edge-soft first:border-t-0">
      {forced ? (
        <div className="flex items-start gap-2 px-4 py-2.5">{heading}</div>
      ) : (
        <button
          type="button"
          onClick={() => state?.toggle(title)}
          aria-expanded={open}
          className="flex w-full items-start gap-2 px-4 py-2.5 text-left transition-colors hover:bg-panel-3/40"
        >
          {heading}
        </button>
      )}

      {open && <div className="divide-y divide-edge-soft px-4 pb-3">{children}</div>}
    </section>
  );
}

export function Row({
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

export function Toggle({
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

function TextField({
  value,
  label,
  placeholder,
  type = "text",
  onCommit,
}: {
  value: string;
  label: string;
  placeholder?: string;
  type?: "text" | "password";
  onCommit: (next: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  return (
    <input
      type={type}
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={() => onCommit(draft)}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
      }}
      placeholder={placeholder}
      aria-label={label}
      spellCheck={false}
      autoComplete="off"
      className="w-44 rounded border border-edge bg-panel-2 px-2 py-1 text-[11px] text-ink placeholder:text-ink-faint focus:border-accent-dim focus:outline-none"
    />
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

export function HotkeyField({
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

/** Keys that only qualify a combination; one pressed alone records nothing. */
const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

/**
 * A readable name for a physical key that the backend still parses.
 *
 * `event.code` is used rather than `event.key` so the combination follows the
 * position of the key instead of the keyboard layout, and the browser's spelling
 * (`KeyR`, `Digit1`, `Numpad1`) is shortened to what the settings show (`R`, `1`,
 * `Num1`).
 */
function keyName(code: string): string {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) return digit[1];
  const numpad = /^Numpad(\d)$/.exec(code);
  if (numpad) return `Num${numpad[1]}`;
  return code;
}

/** The accelerator a key press describes, or null when only modifiers are down. */
function acceleratorOf(event: {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): string | null {
  if (MODIFIER_KEYS.has(event.key) || !event.code) return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  parts.push(keyName(event.code));
  return parts.join("+");
}

/**
 * Press-to-record field for one capture shortcut.
 *
 * Empty is a real value here: it means the action has no shortcut, so Escape
 * cancels a recording and Backspace clears one. Saving is left to the backend,
 * which is the only place that learns a combination another app already owns.
 */
function ShortcutRecorder({
  value,
  label,
  onCommit,
}: {
  value: string;
  /** Accessible name, e.g. "Region shortcut". */
  label: string;
  onCommit: (next: string) => Promise<unknown>;
}) {
  const [recording, setRecording] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const commit = async (next: string) => {
    setRecording(false);
    setProblem(null);
    if (next === value) return;
    if (next) {
      try {
        await validateHotkey(next);
      } catch (e) {
        setProblem(errorMessage(e));
        return;
      }
    }
    // A null answer means the backend refused the combination: the previous one
    // is still stored and still registered, so the field keeps showing it.
    const saved = await onCommit(next);
    if (saved === null) {
      setProblem("The system refused that combination. The previous shortcut is still active.");
    }
  };

  return (
    <div className="flex flex-col items-end gap-0.5">
      <button
        type="button"
        aria-label={`${label} shortcut`}
        title={
          problem ??
          (recording ? "Press the keys; Escape cancels, Backspace clears" : "Click, then press the keys")
        }
        onClick={() => {
          setProblem(null);
          setRecording(true);
        }}
        onBlur={() => setRecording(false)}
        onKeyDown={(event) => {
          if (!recording) return;
          // The rest of the app has keyboard handling of its own; a combination
          // being recorded must not also start a search or copy a clip.
          event.preventDefault();
          event.stopPropagation();
          if (event.key === "Escape") {
            setRecording(false);
            setProblem(null);
            return;
          }
          if (event.key === "Backspace" || event.key === "Delete") {
            void commit("");
            return;
          }
          const combination = acceleratorOf(event);
          if (combination) void commit(combination);
        }}
        className={`w-36 rounded border px-2 py-1 text-center font-mono text-[11px] focus:outline-none ${
          problem
            ? "border-danger bg-panel-2 text-ink"
            : recording
              ? "border-accent bg-accent/10 text-ink"
              : value
                ? "border-edge bg-panel-2 text-ink hover:border-accent-dim"
                : "border-edge bg-panel-2 text-ink-faint hover:border-accent-dim"
        }`}
      >
        {recording ? "Press keys…" : value || "None"}
      </button>
      {problem && <p className="max-w-[14rem] text-right text-[10px] text-danger">{problem}</p>}
    </div>
  );
}

/** Secret expiry, in minutes; 0 is "never", which is the default. */
const SECRET_EXPIRY: { value: number; label: string }[] = [
  { value: 0, label: "Never" },
  { value: 15, label: "15 min" },
  { value: 60, label: "1 hour" },
  { value: 480, label: "8 hours" },
  { value: 1440, label: "1 day" },
];

/** The capture shortcuts, laid out as one column so they read as a set. */
const CAPTURE_SHORTCUTS = [
  { key: "captureRegionHotkey", label: "Region" },
  { key: "captureWindowHotkey", label: "Window" },
  { key: "captureScreenHotkey", label: "Full screen" },
  { key: "captureScrollingHotkey", label: "Scrolling" },
  { key: "pickColorHotkey", label: "Colour picker" },
] as const;

function CaptureShortcutColumn({
  settings,
  onCommit,
}: {
  settings: Settings;
  onCommit: (delta: Partial<Settings>) => Promise<unknown>;
}) {
  return (
    <div className="grid grid-cols-[auto_auto] items-center gap-x-3 gap-y-1">
      {CAPTURE_SHORTCUTS.map(({ key, label }) => (
        <Fragment key={key}>
          <span className="text-right text-[11px] text-ink-faint">{label}</span>
          <ShortcutRecorder
            label={`${label} shortcut`}
            value={settings[key]}
            onCommit={(next) => onCommit({ [key]: next })}
          />
        </Fragment>
      ))}
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

/** Color swatch plus a hex field, so a value can be picked or typed. */
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
        aria-label={`${label} color picker`}
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
      hint="Presets repaint the whole interface; Custom exposes every color."
      control={
        <div className="flex items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
          {modes.map((mode) => (
            <button
              key={mode.id}
              type="button"
              aria-pressed={theme.mode === mode.id}
              onClick={() =>
                // Carry the palette the user is looking at into custom mode so the
                // pickers start from the visible colors.
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

/** Last path segment, for showing a chosen sound without the whole path. */
function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** Explains the update state in one line under the version row. */function updateHint(settings: Settings, state: UpdateState, version: string | null): string {
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
  onRunSetup,
  onOpenChangelog,
  onOpenProject,
  onChooseSound,
  onPatch,
  onDismissError,
  vaultStatus,
  onVaultPrompt,
  onLockVault,
  onRevealDataDir,
  onClearHistory,
  onPrune,
  pausedUntil,
  pausedIndefinitely,
  onPause,
  onResume,
  now,
}: SettingsPanelProps) {
  // Clearing everything is irreversible, so it takes two deliberate clicks.
  const [armed, setArmed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [query, setQuery] = useState("");
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [pendingMove, setPendingMove] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [storageError, setStorageError] = useState<string | null>(null);
  const [markdownError, setMarkdownError] = useState<string | null>(null);
  // Empty on mount, so every section starts collapsed each time settings opens.
  const [openSections, setOpenSections] = useState<string[]>([]);

  const sectionState = useMemo(() => {
    const searching = query.trim().length > 0;
    return {
      forcedOpen: searching,
      isOpen: (title: string) => openSections.includes(title),
      toggle: (title: string) =>
        setOpenSections((prev) =>
          prev.includes(title) ? prev.filter((entry) => entry !== title) : [...prev, title],
        ),
    };
  }, [openSections, query]);

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

  const chooseMarkdownDir = async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose a folder for Markdown notes",
        defaultPath: settings?.markdownExportDir || undefined,
      });
      if (typeof picked === "string") {
        setMarkdownError(null);
        await onPatch({ markdownExportDir: picked });
      }
    } catch (e) {
      setMarkdownError(errorMessage(e));
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
            className={`sticky top-0 z-20 flex items-start gap-2 border-b bg-panel-2 px-4 py-2 text-[11px] shadow-[0_1px_0_0_var(--color-edge-soft)] ${
              error ? "border-danger/40 text-danger" : "border-ok/40 text-ok"
            }`}
          >
            <span
              aria-hidden="true"
              className={`mt-1 size-1.5 shrink-0 rounded-full ${error ? "bg-danger" : "bg-ok"}`}
            />
            <span data-selectable className="min-w-0 flex-1">
              {error ?? notice}
            </span>
            {error && (
              <button
                type="button"
                onClick={() => {
                  copyText(error)
                    .then(() => {
                      setCopied(true);
                      window.setTimeout(() => setCopied(false), 1_500);
                    })
                    .catch(() => undefined);
                }}
                title="Copy this message"
                className="shrink-0 underline underline-offset-2"
              >
                {copied ? "Copied" : "Copy"}
              </button>
            )}
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

        <SettingsSections query={query} sectionState={sectionState}>
          <Section
            title="About"
            hint="Signed installers come from the project's GitHub releases."
          >
            <Row
              label="Version"
              hint={updateHint(settings, updateState, appVersion)}
              control={
                <div className="flex items-center gap-1.5">
                  {updateState.kind === "available" && (
                    <button
                      type="button"
                      onClick={() => onOpenChangelog(updateState.version)}
                      className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                    >
                      What&apos;s new
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={onCheckForUpdates}
                    disabled={updateState.kind === "checking" || updateState.kind === "downloading"}
                    className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                  >
                    {updateState.kind === "checking" ? "Checking…" : "Check now"}
                  </button>
                </div>
              }
            />
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
              label="GitHub project"
              hint="Source, issues and the full release history."
              control={
                <button
                  type="button"
                  onClick={onOpenProject}
                  className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                >
                  Open
                </button>
              }
            />
            <Row
              label="Setup wizard"
              hint="Walks through the hotkey, paste, capture and privacy choices again."
              control={
                <button
                  type="button"
                  onClick={onRunSetup}
                  className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                >
                  Run setup
                </button>
              }
            />
          </Section>

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
            label="Title bar capture icons"
            hint="The eyedropper and screenshot buttons. Off leaves the same tools in the sidebar strip, and the capture hotkeys keep working."
            control={
              <Toggle
                label="Show title bar capture icons"
                checked={settings.showTitlebarCaptureIcons}
                onChange={(next) => void onPatch({ showTitlebarCaptureIcons: next })}
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
            label="Colors"
            hint="Cards are the default here too."
            control={
              <ViewModeField
                label="Colors layout"
                value={settings.viewModes.colors}
                onChange={(mode) =>
                  void onPatch({ viewModes: { ...settings.viewModes, colors: mode } })
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
            label="Sidebar lists"
            hint="Archives show this many before “Show all”; tags stop at 10."
            control={
              <NumberField
                value={settings.sidebarListLimit}
                min={3}
                max={50}
                suffix="items"
                onChange={(next) => void onPatch({ sidebarListLimit: next })}
              />
            }
          />
          <Row
            label="Card size"
            hint="Sets every history view; the toolbar slider adjusts only the current one."
            control={
              <NumberField
                value={Math.round(settings.gridScale * 100)}
                min={60}
                max={200}
                suffix="%"
                onChange={(next) =>
                  void onPatch({ gridScale: next / 100, gridScales: {} })
                }
              />
            }
          />
          <Row
            label="GIF preview size"
            hint="Mirrors the slider in Utilities → My GIFs."
            control={
              <NumberField
                value={Math.round(settings.gifScale * 100)}
                min={60}
                max={200}
                suffix="%"
                onChange={(next) => void onPatch({ gifScale: next / 100 })}
              />
            }
          />
          <Row
            label="Emoji preview size"
            hint="Mirrors the slider in Utilities → Emoji."
            control={
              <NumberField
                value={Math.round(settings.emojiScale * 100)}
                min={60}
                max={200}
                suffix="%"
                onChange={(next) => void onPatch({ emojiScale: next / 100 })}
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
          <Row
            label="Preview text size"
            hint="Text clips in the quick preview. The preview's own − and + buttons change the same value; 100% is the designed size."
            control={
              <div className="flex items-center gap-1">
                {[70, 85, 100, 125, 150, 200].map((percent) => (
                  <button
                    key={percent}
                    type="button"
                    aria-pressed={settings.previewTextScale === percent}
                    onClick={() => void onPatch({ previewTextScale: percent })}
                    className={`rounded border px-1.5 py-0.5 text-[10px] tabular-nums transition-colors ${
                      settings.previewTextScale === percent
                        ? "border-accent bg-accent/20 text-accent-hi"
                        : "border-edge bg-panel-2 text-ink-dim hover:bg-panel-3 hover:text-ink"
                    }`}
                  >
                    {percent}%
                  </button>
                ))}
              </div>
            }
          />
          <Row
            label="Capture outline color"
            hint="The border, window highlight, size chip and colour-picker crosshair. Chrome only: it is never part of the captured image."
            control={
              <div className="flex items-center gap-1.5">
                <span
                  aria-hidden
                  title="The outline as the overlay draws it"
                  className="size-6 shrink-0 rounded bg-panel-2"
                  style={{ border: `2px solid ${settings.captureOutline}` }}
                />
                <ColorField
                  label="Capture outline"
                  value={settings.captureOutline}
                  onChange={(next) => void onPatch({ captureOutline: next })}
                />
                <button
                  type="button"
                  onClick={() => void onPatch({ captureOutline: DEFAULT_CAPTURE_OUTLINE })}
                  disabled={settings.captureOutline === DEFAULT_CAPTURE_OUTLINE}
                  title={`Back to ${DEFAULT_CAPTURE_OUTLINE}`}
                  className="rounded border border-edge bg-panel-2 px-2 py-1 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
                >
                  Reset
                </button>
              </div>
            }
          />
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
              label="Private mode shortcut"
              hint="Toggles capture on and off without opening the window. Capture is also paused from the tray menu."
              control={
                <HotkeyField
                  value={settings.pauseHotkey}
                  onCommit={(next) => void onPatch({ pauseHotkey: next })}
                />
              }
            />
            <Row
              label="Private mode"
              hint={
                pausedIndefinitely
                  ? "Capture is paused until CopyPaste restarts."
                  : pausedUntil !== null && pausedUntil > now
                    ? `Capture resumes in ${Math.max(1, Math.round((pausedUntil - now) / 60_000))} minute(s).`
                    : "Copies are recorded normally."
              }
              control={
                <div className="flex flex-wrap justify-end gap-1">
                  {[
                    { label: "15 min", minutes: 15, untilRestart: false },
                    { label: "1 hour", minutes: 60, untilRestart: false },
                    { label: "Until restart", minutes: 0, untilRestart: true },
                  ].map((option) => (
                    <button
                      key={option.label}
                      type="button"
                      onClick={() => onPause(option.minutes, option.untilRestart)}
                      className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:border-accent-dim hover:text-ink"
                    >
                      {option.label}
                    </button>
                  ))}
                  <button
                    type="button"
                    onClick={onResume}
                    className="rounded border border-accent bg-accent/20 px-2 py-0.5 text-[10px] font-medium text-accent-hi transition-colors hover:bg-accent/30"
                  >
                    Resume
                  </button>
                </div>
              }
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
              label="Close after double-click"
              hint="Double-clicking a clip copies it and hides the overlay. When off, the clip is still copied but the window stays open."
              control={
                <Toggle
                  label="Close after double-click"
                  checked={settings.closeOnDoubleClick}
                  onChange={(next) => void onPatch({ closeOnDoubleClick: next })}
                />
              }
            />
            <Row
              label="Paste with formatting"
              hint="Keeps the HTML and RTF a source app offered, so a styled copy pastes back styled. The plain text is always kept too."
              control={
                <Toggle
                  label="Paste with formatting"
                  checked={settings.pasteRichText}
                  onChange={(next) => void onPatch({ pasteRichText: next })}
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
            <Row
              label="Clear the clipboard after use"
              hint="Empties the clipboard once a clip has been used. Anything copied in the meantime is left alone."
              control={
                <Toggle
                  label="Clear the clipboard after use"
                  checked={settings.autoClearClipboard}
                  onChange={(next) => void onPatch({ autoClearClipboard: next })}
                />
              }
            />
            <Row
              label="Clear after"
              hint="0 empties it as soon as the window hides; a longer delay leaves time to paste."
              control={
                <NumberField
                  value={settings.autoClearDelaySecs}
                  min={0}
                  max={3600}
                  suffix="secs"
                  onChange={(next) => void onPatch({ autoClearDelaySecs: next })}
                />
              }
            />
          </Section>

          <Section title="Window">
            <Row
              label="Always on top"
              hint="Keeps the window above other applications. Off by default."
              control={
                <Toggle
                  label="Always on top"
                  checked={settings.alwaysOnTop}
                  onChange={(next) => void onPatch({ alwaysOnTop: next })}
                />
              }
            />
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
              label="Capture shortcuts"
              hint="Region, window, full-screen, scrolling and the colour picker can each have their own combination. Click a field and press the keys; leave it empty for none."
              control={<CaptureShortcutColumn settings={settings} onCommit={onPatch} />}
            />
            <Row
              label="Default capture"
              hint="What the header's capture button, and Print Screen when it is enabled, start."
              control={
                <CaptureModeField
                  label="Default capture type"
                  value={settings.captureMode}
                  onChange={(next) => void onPatch({ captureMode: next })}
                />
              }
            />
            <Row
              label="Print Screen"
              hint="Captures the default type above. Windows gives the key to the Snipping Tool, so turning this on switches that off first; turning it off puts it back exactly as it was."
              control={
                <Toggle
                  label="Print Screen captures"
                  checked={settings.printScreenCapture}
                  onChange={(next) => void onPatch({ printScreenCapture: next })}
                />
              }
            />
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
              label="Detect faces"
              hint="Runs a local detector so image clips can be filtered as People. Nothing leaves the machine."
              control={
                <Toggle
                  label="Detect faces"
                  checked={settings.detectFaces}
                  onChange={(next) => void onPatch({ detectFaces: next })}
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
            <Row
              label="Flag likely secrets"
              hint="Marks clips that look like API keys, tokens or private keys. Detection stays on this machine."
              control={
                <Toggle
                  label="Flag likely secrets"
                  checked={settings.flagSecrets}
                  onChange={(next) => void onPatch({ flagSecrets: next })}
                />
              }
            />
            <Row
              label="Guess from context"
              hint="Also flags keyword-and-entropy matches. Slightly noisier, so it is off by default."
              control={
                <Toggle
                  label="Guess from context"
                  checked={settings.secretHeuristics}
                  onChange={(next) => void onPatch({ secretHeuristics: next })}
                />
              }
            />
            <Row
              label="Leave secrets out of history"
              hint="Skips a flagged copy entirely instead of storing it blurred."
              control={
                <Toggle
                  label="Leave secrets out of history"
                  checked={settings.skipSecretCapture}
                  onChange={(next) => void onPatch({ skipSecretCapture: next })}
                />
              }
            />
            <Row
              label="Secret vault"
              hint={
                vaultStatus === "none"
                  ? "No vault yet. Marking a clip secret creates one, or make it now."
                  : vaultStatus === "locked"
                    ? "Sealed clips read as empty until the vault is unlocked. A restart always comes back locked."
                    : "Open for this run. Locking forgets the key; the clips stay encrypted."
              }
              control={
                <div className="flex flex-wrap justify-end gap-1">
                  {vaultStatus === "none" && (
                    <button
                      type="button"
                      onClick={() => onVaultPrompt("create")}
                      className="rounded border border-accent bg-accent/20 px-2 py-0.5 text-[10px] font-medium text-accent-hi transition-colors hover:bg-accent/30"
                    >
                      Create vault
                    </button>
                  )}
                  {vaultStatus === "locked" && (
                    <button
                      type="button"
                      onClick={() => onVaultPrompt("unlock")}
                      className="rounded border border-accent bg-accent/20 px-2 py-0.5 text-[10px] font-medium text-accent-hi transition-colors hover:bg-accent/30"
                    >
                      Unlock
                    </button>
                  )}
                  {vaultStatus !== "none" && (
                    <>
                      <button
                        type="button"
                        onClick={() => onVaultPrompt("change")}
                        className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                      >
                        Change passphrase
                      </button>
                      {vaultStatus === "unlocked" && (
                        <button
                          type="button"
                          onClick={onLockVault}
                          className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                        >
                          Lock now
                        </button>
                      )}
                    </>
                  )}
                </div>
              }
            />
            <Row
              label="Secrets expire after"
              hint="Applies to clips marked secret. Pinning one or filing it in a collection does not keep it past this."
              control={
                <div className="flex flex-wrap justify-end gap-1">
                  {SECRET_EXPIRY.map((option) => (
                    <button
                      key={option.value}
                      type="button"
                      aria-pressed={settings.secretMaxAgeMins === option.value}
                      onClick={() => void onPatch({ secretMaxAgeMins: option.value })}
                      className={`rounded border px-2 py-0.5 text-[10px] transition-colors ${
                        settings.secretMaxAgeMins === option.value
                          ? "border-accent bg-accent/20 text-accent-hi"
                          : "border-edge bg-panel-2 text-ink-dim hover:bg-panel-3 hover:text-ink"
                      }`}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
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
            title="Utilities"
            hint="Optional keys for the built-in tools. Nothing is sent anywhere until you use them."
          >
            <Row
              label="GIPHY API key"
              hint="Required for GIF search. A free personal key takes a minute at developers.giphy.com and stays in your local settings."
              control={
                <TextField
                  type="password"
                  label="GIPHY API key"
                  value={settings.giphyApiKey}
                  placeholder="Paste your key"
                  onCommit={(next) => void onPatch({ giphyApiKey: next })}
                />
              }
            />
          </Section>

          <Section
            title="Markdown"
            hint="Export a text clip as a note for Obsidian or another Markdown vault."
          >
            <Row
              label="Export folder"
              hint={settings.markdownExportDir || "Ask for a folder each time."}
              control={
                <div className="flex gap-1.5">
                  <button
                    type="button"
                    onClick={() => void chooseMarkdownDir()}
                    className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                  >
                    Choose…
                  </button>
                  {settings.markdownExportDir && (
                    <button
                      type="button"
                      onClick={() => void onPatch({ markdownExportDir: "" })}
                      className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                    >
                      Clear
                    </button>
                  )}
                </div>
              }
            />
            {markdownError && (
              <p className="py-2 text-[10px] leading-relaxed text-danger">{markdownError}</p>
            )}
            <Row
              label="YAML front matter"
              hint="Adds created, source, tags and the clip id to the top of each note."
              control={
                <Toggle
                  label="YAML front matter"
                  checked={settings.markdownFrontMatter}
                  onChange={(next) => void onPatch({ markdownFrontMatter: next })}
                />
              }
            />
            <Row
              label="Date in filenames"
              hint="Prefixes each note with the capture date, YYYY-MM-DD."
              control={
                <Toggle
                  label="Date in filenames"
                  checked={settings.markdownDatePrefix}
                  onChange={(next) => void onPatch({ markdownDatePrefix: next })}
                />
              }
            />
          </Section>

          <Section
            title="Sounds"
            hint="Feedback for copy outcomes; leave the file empty for the built-in sound."
          >
            <Row
              label="Copy sound"
              hint={settings.copySoundFile ? fileName(settings.copySoundFile) : "A short built-in tick."}
              control={
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    onClick={() => onChooseSound("copy")}
                    className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                  >
                    {settings.copySoundFile ? "Change…" : "Choose WAV…"}
                  </button>
                  {settings.copySoundFile && (
                    <button
                      type="button"
                      onClick={() => void onPatch({ copySoundFile: "" })}
                      className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                    >
                      Default
                    </button>
                  )}
                  <Toggle
                    label="Copy sound"
                    checked={settings.copySound}
                    onChange={(next) => void onPatch({ copySound: next })}
                  />
                </div>
              }
            />
            <Row
              label="Failure sound"
              hint={
                settings.copyFailSoundFile
                  ? fileName(settings.copyFailSoundFile)
                  : "A built-in lower tone, played when a copy fails."
              }
              control={
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    onClick={() => onChooseSound("fail")}
                    className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                  >
                    {settings.copyFailSoundFile ? "Change…" : "Choose WAV…"}
                  </button>
                  {settings.copyFailSoundFile && (
                    <button
                      type="button"
                      onClick={() => void onPatch({ copyFailSoundFile: "" })}
                      className="rounded border border-edge bg-panel-2 px-2 py-0.5 text-[10px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
                    >
                      Default
                    </button>
                  )}
                  <Toggle
                    label="Failure sound"
                    checked={settings.copyFailSound}
                    onChange={(next) => void onPatch({ copyFailSound: next })}
                  />
                </div>
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
          <p className="text-[10px] leading-relaxed text-ink-faint">
            History is stored locally in plain text in the data folder above. Nothing is ever sent
            anywhere; back up that folder if you want a copy of your clips.
          </p>
        </section>
      </div>
    </div>
  );
}
