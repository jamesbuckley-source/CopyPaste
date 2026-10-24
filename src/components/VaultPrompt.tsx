import { useEffect, useRef, useState } from "react";

import { errorMessage } from "../lib/ipc";
import { IconClose, IconLock, IconShield } from "./icons";

/**
 * Asks for the vault passphrase.
 *
 * One window for all four jobs, because they are all "the vault needs a
 * passphrase" and a user should not have to learn four dialogs: create it, open
 * it, change the phrase, or confirm the current one before locking.
 */
export type VaultPromptKind = "create" | "unlock" | "change";

interface VaultPromptProps {
  kind: VaultPromptKind;
  /** Why the dialog appeared, so it is not a mystery. */
  reason?: string;
  onSubmit: (values: { passphrase: string; next?: string }) => Promise<void>;
  onClose: () => void;
}

const TITLES: Record<VaultPromptKind, string> = {
  create: "Create the vault",
  unlock: "Unlock the vault",
  change: "Change the vault passphrase",
};

const HINTS: Record<VaultPromptKind, string> = {
  create:
    "Clips you mark as secret are encrypted with this passphrase. There is no recovery: if it is lost, those clips cannot be read again.",
  unlock: "Secret clips are encrypted. Enter the passphrase to read them.",
  change:
    "The clips are not re-encrypted, only the key that protects them, so this is instant and safe to do at any time.",
};

/** Ten characters is the point where a passphrase stops being a word. */
const MIN_NEW_LENGTH = 10;

export function VaultPrompt({ kind, reason, onSubmit, onClose }: VaultPromptProps) {
  const [passphrase, setPassphrase] = useState("");
  const [next, setNext] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const first = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    first.current?.focus();
  }, []);

  const creating = kind === "create";
  const mismatch = creating && confirmation.length > 0 && confirmation !== next;
  const tooShort = creating && next.length > 0 && next.length < MIN_NEW_LENGTH;
  const ready =
    creating
      ? next.length >= MIN_NEW_LENGTH && confirmation === next
      : passphrase.trim().length > 0 && (kind !== "change" || next.length >= MIN_NEW_LENGTH);

  const submit = async () => {
    if (!ready || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(creating ? { passphrase: next } : { passphrase, next: kind === "change" ? next : undefined });
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-canvas/80 p-6 animate-fade-in"
      onMouseDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="vault-prompt-title"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
        className="w-full max-w-[420px] rounded-panel border border-edge bg-panel p-4 shadow-xl animate-rise"
      >
        <div className="flex items-start gap-2">
          <IconShield className="mt-0.5 size-4 shrink-0 text-accent" />
          <div className="min-w-0 flex-1">
            <h2 id="vault-prompt-title" className="text-[12px] font-medium text-ink">
              {TITLES[kind]}
            </h2>
            <p className="mt-1 text-[10px] leading-relaxed text-ink-faint">{HINTS[kind]}</p>
            {reason && (
              <p className="mt-1.5 rounded border border-edge-soft bg-panel-2 px-2 py-1 text-[10px] leading-relaxed text-ink-dim">
                {reason}
              </p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            title="Close"
            aria-label="Close"
            className="rounded p-1 text-ink-faint transition-colors hover:bg-panel-3 hover:text-ink"
          >
            <IconClose className="size-3.5" />
          </button>
        </div>

        <div className="mt-3 flex flex-col gap-2">
          {kind === "change" && (
            <label className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wide text-ink-faint">Current passphrase</span>
              <input
                ref={first}
                type="password"
                value={passphrase}
                onChange={(event) => setPassphrase(event.target.value)}
                autoComplete="current-password"
                className="rounded border border-edge bg-panel-2 px-2 py-1.5 text-[12px] text-ink focus:border-accent-dim focus:outline-none"
              />
            </label>
          )}

          {(creating || kind === "change") && (
            <>
              <label className="flex flex-col gap-1">
                <span className="text-[10px] uppercase tracking-wide text-ink-faint">
                  {creating ? "Passphrase" : "New passphrase"}
                </span>
                <input
                  ref={creating ? first : undefined}
                  type="password"
                  value={next}
                  onChange={(event) => setNext(event.target.value)}
                  autoComplete="new-password"
                  className="rounded border border-edge bg-panel-2 px-2 py-1.5 text-[12px] text-ink focus:border-accent-dim focus:outline-none"
                />
              </label>
              {tooShort && (
                <p className="text-[10px] text-warn">At least {MIN_NEW_LENGTH} characters.</p>
              )}
              <label className="flex flex-col gap-1">
                <span className="text-[10px] uppercase tracking-wide text-ink-faint">Confirm</span>
                <input
                  type="password"
                  value={confirmation}
                  onChange={(event) => setConfirmation(event.target.value)}
                  autoComplete="new-password"
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void submit();
                  }}
                  className="rounded border border-edge bg-panel-2 px-2 py-1.5 text-[12px] text-ink focus:border-accent-dim focus:outline-none"
                />
              </label>
              {mismatch && <p className="text-[10px] text-danger">Those do not match.</p>}
            </>
          )}

          {kind === "unlock" && (
            <label className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wide text-ink-faint">Passphrase</span>
              <input
                ref={first}
                type="password"
                value={passphrase}
                onChange={(event) => setPassphrase(event.target.value)}
                autoComplete="current-password"
                onKeyDown={(event) => {
                  if (event.key === "Enter") void submit();
                }}
                className="rounded border border-edge bg-panel-2 px-2 py-1.5 text-[12px] text-ink focus:border-accent-dim focus:outline-none"
              />
            </label>
          )}
        </div>

        {error && (
          <p role="alert" className="mt-2 rounded border border-danger/40 bg-danger/10 px-2 py-1.5 text-[10px] leading-relaxed text-danger">
            {error}
          </p>
        )}

        <div className="mt-3 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-edge bg-panel-2 px-2.5 py-1 text-[11px] text-ink-dim transition-colors hover:bg-panel-3 hover:text-ink"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void submit()}
            disabled={!ready || busy || mismatch}
            className="flex items-center gap-1.5 rounded border border-accent bg-accent/20 px-2.5 py-1 text-[11px] font-medium text-accent-hi transition-colors hover:bg-accent/30 disabled:opacity-40"
          >
            <IconLock className="size-3" />
            {busy ? "Working…" : creating ? "Create vault" : kind === "change" ? "Change it" : "Unlock"}
          </button>
        </div>
      </div>
    </div>
  );
}
