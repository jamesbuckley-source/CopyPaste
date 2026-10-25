import { createContext, useContext } from "react";

/**
 * Whether sealed clips can be read right now, and the way to ask for the
 * passphrase.
 *
 * A context rather than a prop: the reveal button lives inside a card, a row and
 * the detail pane, and threading a callback through all three would put a vault
 * concern into components that otherwise know nothing about it.
 */
export interface VaultGate {
  locked: boolean;
  /** Opens the passphrase window. Does nothing on a build with no vault. */
  requestUnlock: () => void;
}

const CLOSED: VaultGate = { locked: false, requestUnlock: () => undefined };

const VaultGateContext = createContext<VaultGate>(CLOSED);

export const VaultGateProvider = VaultGateContext.Provider;

export function useVaultGate(): VaultGate {
  return useContext(VaultGateContext);
}
