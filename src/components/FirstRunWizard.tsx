import { useEffect, useState, type ReactNode } from "react";

import { errorMessage, validateHotkey } from "../lib/ipc";
import type { Settings, ThemeMode } from "../lib/types";
import { HotkeyField, Row, Toggle } from "./SettingsPanel";
import { IconChevronLeft, IconClose } from "./icons";

/** Process names the wizard offers to keep out of history. */
const PASSWORD_MANAGERS = [
  "keepassxc.exe",
  "bitwarden.exe",
  "1password.exe",
  "dashlane.exe",
];

type Retention = "500" | "2000" | "30d" | "90d" | "forever";
type SecretMode = "flag" | "skip" | "off";

const RETENTION_PATCH: Record<
  Retention,
  Pick<Settings, "maxItems" | "maxAgeDays" | "maxAgeHours">
> = {
  "500": { maxItems: 500, maxAgeDays: 0, maxAgeHours: 0 },
  "2000": { maxItems: 2000, maxAgeDays: 0, maxAgeHours: 0 },
  "30d": { maxItems: 0, maxAgeDays: 30, maxAgeHours: 0 },
  "90d": { maxItems: 0, maxAgeDays: 90, maxAgeHours: 0 },
  forever: { maxItems: 0, maxAgeDays: 0, maxAgeHours: 0 },
};

const RETENTION_LABELS: { value: Retention; label: string }[] = [
  { value: "500", label: "500 clips" },
  { value: "2000", label: "2,000 clips" },
  { value: "30d", label: "30 days" },
  { value: "90d", label: "90 days" },
  { value: "forever", label: "Forever" },
];

const SECRET_OPTIONS: { value: SecretMode; label: string; hint: string }[] = [
  {
    value: "flag",
    label: "Flag and blur",
    hint: "Detected credentials stay in history behind a Secret label.",
  },
  {
    value: "skip",
    label: "Leave out of history",
    hint: "Flagged copies are never stored at all.",
  },
  {
    value: "off",
    label: "Do not detect",
    hint: "Secrets are captured like any other text.",
  },
];

function initialRetention(settings: Settings): Retention {
  if (settings.maxAgeDays === 30) return "30d";
  if (settings.maxAgeDays === 90) return "90d";
  if (settings.maxAgeDays === 0 && settings.maxAgeHours === 0) {
    if (settings.maxItems === 500) return "500";
    if (settings.maxItems === 2000) return "2000";
    if (settings.maxItems === 0) return "forever";
  }
  return "2000";
}

function initialSecretMode(settings: Settings): SecretMode {
  if (!settings.flagSecrets) return "off";
  return settings.skipSecretCapture ? "skip" : "flag";
}

function ignoresPasswordManagers(settings: Settings): boolean {
  return PASSWORD_MANAGERS.every((app) => settings.ignoredApps.includes(app));
}

function initialThemeMode(settings: Settings): ThemeMode {
  return settings.theme.mode === "light" ? "light" : "dark";
}

interface FirstRunWizardProps {
  settings: Settings;
  /** True when the wizard opens automatically, false when re-run from Settings. */
  firstRun: boolean;
  onApply: (delta: Partial<Settings>) => Promise<Settings>;
  onClose: () => void;
  /** Saves the choices, closes the wizard and focuses the search box. */
  onFinish: () => void;
}

function OptionButton({
  selected,
  onClick,
  children,
}: {
  selected: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={`rounded border px-2.5 py-1 text-[11px] transition-colors ${
        selected
          ? "border-accent bg-accent/20 text-accent-hi"
          : "border-edge bg-panel-2 text-ink-dim hover:bg-panel-3 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

function StepTitle({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="mb-3">
      <h2 className="text-[13px] font-semibold text-ink">{title}</h2>
      <p className="mt-0.5 text-[11px] leading-relaxed text-ink-faint">{hint}</p>
    </div>
  );
}

export function FirstRunWizard({
  settings,
  firstRun,
  onApply,
  onClose,
  onFinish,
}: FirstRunWizardProps) {
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [hotkey, setHotkey] = useState(settings.hotkey);
  const [launchAtLogin, setLaunchAtLogin] = useState(
    firstRun ? true : settings.launchAtLogin,
  );
  const [autoPaste, setAutoPaste] = useState(firstRun ? true : settings.autoPaste);
  const [captureMedia, setCaptureMedia] = useState(
    settings.captureImages || settings.captureFiles,
  );
  const [ignoreManagers, setIgnoreManagers] = useState(
    firstRun ? true : ignoresPasswordManagers(settings),
  );
  const [secretMode, setSecretMode] = useState<SecretMode>(initialSecretMode(settings));
  const [retention, setRetention] = useState<Retention>(initialRetention(settings));
  const [themeMode, setThemeMode] = useState<ThemeMode>(initialThemeMode(settings));
  const [copySound, setCopySound] = useState(settings.copySound);
  const [failureSound, setFailureSound] = useState(settings.copyFailSound);

  const initialTheme = initialThemeMode(settings);
  // Re-running the wizard must not normalise choices the user made in Settings:
  // only fields the user actually touched are written back.
  const initialCapture = settings.captureImages || settings.captureFiles;
  const initialIgnore = firstRun ? true : ignoresPasswordManagers(settings);
  const initialRetentionValue = initialRetention(settings);

  const skip = async () => {
    if (!firstRun) {
      onClose();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onApply({ setupComplete: true });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      if (!busy) void skip();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [busy, firstRun, onApply, onClose]);

  const back = () => {
    setError(null);
    setStep((current) => Math.max(0, current - 1));
  };

  const advance = async () => {
    setError(null);
    if (step === 1) {
      setStep(2);
      return;
    }
    if (step !== 0) return;

    setBusy(true);
    try {
      const trimmed = hotkey.trim();
      if (trimmed && trimmed !== settings.hotkey) {
        await validateHotkey(trimmed);
      }
      // Registering the summon shortcut is the only way to know the OS will
      // accept it, so apply it before moving on instead of failing at the end.
      await onApply({ hotkey: trimmed || settings.hotkey, launchAtLogin });
      setStep(1);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      const delta: Partial<Settings> = {
        setupComplete: true,
        autoPaste,
        flagSecrets: secretMode !== "off",
        skipSecretCapture: secretMode === "skip",
        copySound,
        copyFailSound: failureSound,
      };

      if (firstRun || captureMedia !== initialCapture) {
        delta.captureImages = captureMedia;
        delta.captureFiles = captureMedia;
      }

      if (firstRun || ignoreManagers !== initialIgnore) {
        delta.ignoredApps = ignoreManagers
          ? Array.from(new Set([...settings.ignoredApps, ...PASSWORD_MANAGERS]))
          : settings.ignoredApps.filter((app) => !PASSWORD_MANAGERS.includes(app));
      }

      if (firstRun || retention !== initialRetentionValue) {
        Object.assign(delta, RETENTION_PATCH[retention]);
      }

      // Leave a custom palette alone when the wizard is only being re-run.
      if (firstRun || themeMode !== initialTheme) {
        delta.theme = { ...settings.theme, mode: themeMode };
      }

      await onApply(delta);
      onFinish();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  const steps = ["Activation", "Paste & privacy", "Look & finish"];

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-canvas/80 p-6 animate-fade-in">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="setup-title"
        className="flex w-full max-w-[480px] flex-col overflow-hidden rounded-panel border border-edge bg-panel shadow-xl animate-rise"
      >
        <div className="flex items-center gap-2 border-b border-edge-soft px-5 py-3">
          <div className="min-w-0 flex-1">
            <h2 id="setup-title" className="text-[13px] font-semibold text-ink">
              Set up CopyPaste
            </h2>
            <p className="text-[10px] text-ink-faint">
              Three quick choices. Everything can be changed later in Settings.
            </p>
          </div>
          <button
            type="button"
            onClick={() => void skip()}
            disabled={busy}
            title={firstRun ? "Skip setup" : "Close"}
            aria-label={firstRun ? "Skip setup" : "Close"}
            className="flex size-6 shrink-0 items-center justify-center rounded text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
          >
            <IconClose className="size-3.5" />
          </button>
        </div>

        <div className="flex gap-1 px-5 pt-3" aria-hidden="true">
          {steps.map((label, index) => (
            <div
              key={label}
              className={`h-0.5 flex-1 rounded-full transition-colors ${
                index <= step ? "bg-accent" : "bg-panel-3"
              }`}
            />
          ))}
        </div>

        <div className="min-h-[240px] px-5 py-3">
          {error && (
            <p role="alert" className="mb-3 rounded border border-danger/40 bg-danger/10 px-2 py-1.5 text-[11px] text-danger">
              {error}
            </p>
          )}

          {step === 0 && (
            <>
              <StepTitle
                title="Activation"
                hint="The shortcut that summons the window from anywhere. It must be unique across Windows."
              />
              <Row
                label="Global hotkey"
                hint="Written with a plus sign, for example Alt+V or Ctrl+Shift+V."
                control={<HotkeyField value={settings.hotkey} onCommit={setHotkey} />}
              />
              <Row
                label="Start with Windows"
                hint="Runs quietly in the tray from sign-in and captures every copy."
                control={
                  <Toggle
                    label="Start with Windows"
                    checked={launchAtLogin}
                    onChange={setLaunchAtLogin}
                  />
                }
              />
            </>
          )}

          {step === 1 && (
            <>
              <StepTitle
                title="Paste & privacy"
                hint="Choose what gets recorded and how clips come back out."
              />
              <Row
                label="Paste automatically"
                hint="Sends Ctrl+V to the window you were in after you pick a clip."
                control={
                  <Toggle
                    label="Paste automatically"
                    checked={autoPaste}
                    onChange={setAutoPaste}
                  />
                }
              />
              <Row
                label="Capture images and files"
                hint="Adds screenshots and copied File Explorer paths to history."
                control={
                  <Toggle
                    label="Capture images and files"
                    checked={captureMedia}
                    onChange={setCaptureMedia}
                  />
                }
              />
              <Row
                label="Ignore password managers"
                hint="Never records copies made in KeePassXC, Bitwarden, 1Password or Dashlane."
                control={
                  <Toggle
                    label="Ignore password managers"
                    checked={ignoreManagers}
                    onChange={setIgnoreManagers}
                  />
                }
              />

              <div className="py-2">
                <p className="text-[12px] text-ink-dim">Likely secrets</p>
                <p className="mt-0.5 text-[10px] leading-relaxed text-ink-faint">
                  Detection stays on this machine.
                </p>
                <div className="mt-1.5 flex flex-col gap-1">
                  {SECRET_OPTIONS.map((option) => (
                    <button
                      key={option.value}
                      type="button"
                      aria-pressed={secretMode === option.value}
                      onClick={() => setSecretMode(option.value)}
                      className={`rounded border px-2.5 py-1.5 text-left transition-colors ${
                        secretMode === option.value
                          ? "border-accent bg-accent/15"
                          : "border-edge bg-panel-2 hover:bg-panel-3"
                      }`}
                    >
                      <span className="block text-[11px] text-ink">{option.label}</span>
                      <span className="block text-[10px] text-ink-faint">{option.hint}</span>
                    </button>
                  ))}
                </div>
              </div>

              <div className="py-2">
                <p className="text-[12px] text-ink-dim">Keep history for</p>
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {RETENTION_LABELS.map((option) => (
                    <OptionButton
                      key={option.value}
                      selected={retention === option.value}
                      onClick={() => setRetention(option.value)}
                    >
                      {option.label}
                    </OptionButton>
                  ))}
                </div>
              </div>
            </>
          )}

          {step === 2 && (
            <>
              <StepTitle title="Look & finish" hint="A theme and two sounds, then you are done." />
              <Row
                label="Theme"
                hint="Custom colors stay available in Settings → Appearance."
                control={
                  <div className="flex items-center gap-0.5 rounded border border-edge bg-panel-2 p-0.5">
                    {(["dark", "light"] as const).map((mode) => (
                      <button
                        key={mode}
                        type="button"
                        aria-pressed={themeMode === mode}
                        onClick={() => setThemeMode(mode)}
                        className={`rounded px-2 py-0.5 text-[10px] capitalize transition-colors ${
                          themeMode === mode
                            ? "bg-accent/25 text-accent-hi"
                            : "text-ink-faint hover:text-ink-dim"
                        }`}
                      >
                        {mode}
                      </button>
                    ))}
                  </div>
                }
              />
              <Row
                label="Copy sound"
                hint="A short tick when a copy lands in history."
                control={
                  <Toggle label="Copy sound" checked={copySound} onChange={setCopySound} />
                }
              />
              <Row
                label="Failure sound"
                hint="A lower tone when a copy cannot be recorded."
                control={
                  <Toggle
                    label="Failure sound"
                    checked={failureSound}
                    onChange={setFailureSound}
                  />
                }
              />
              <p className="mt-3 rounded border border-edge-soft bg-panel-2 px-2.5 py-2 text-[10px] leading-relaxed text-ink-faint">
                You can reopen this wizard any time from Settings → About → Setup wizard.
              </p>
            </>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-edge-soft px-5 py-3">
          <button
            type="button"
            onClick={() => void skip()}
            disabled={busy}
            className="text-[11px] text-ink-faint underline underline-offset-2 transition-colors hover:text-ink disabled:opacity-40"
          >
            {firstRun ? "Skip setup" : "Cancel"}
          </button>

          <div className="ml-auto flex items-center gap-1.5">
            {step > 0 && (
              <button
                type="button"
                onClick={back}
                disabled={busy}
                className="flex items-center gap-1 rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink disabled:opacity-40"
              >
                <IconChevronLeft className="size-3" />
                Back
              </button>
            )}
            {step < 2 ? (
              <button
                type="button"
                autoFocus
                onClick={() => void advance()}
                disabled={busy}
                className="rounded border border-accent-dim bg-accent/20 px-3 py-1 text-[11px] font-medium text-accent-hi transition-colors hover:bg-accent/30 disabled:opacity-40"
              >
                {busy ? "Saving…" : "Next"}
              </button>
            ) : (
              <button
                type="button"
                autoFocus
                onClick={() => void finish()}
                disabled={busy}
                className="rounded border border-accent-dim bg-accent/20 px-3 py-1 text-[11px] font-medium text-accent-hi transition-colors hover:bg-accent/30 disabled:opacity-40"
              >
                {busy ? "Saving…" : "Finish"}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
