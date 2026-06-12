import type { AccountSummary } from "../types";

/**
 * Account → target-binding derivation, shared by the shell (App.tsx).
 *
 * This file used to also export a `useAccounts` hook carrying its own
 * syncFromCurrentCc + focus-debounced refresh pipeline. That hook was
 * the source of the double-traffic bug AppStateProvider's header
 * describes (two parallel refresh pipelines doubling `/profile` and
 * `verify_all_accounts` calls) and was deleted once unused — account
 * list + refresh state now live solely in AppStateProvider.
 */

export interface TargetBinding {
  cli: string | null;
  desktop: string | null;
}

/** Derive `{cli, desktop}` UUID binding from the active flags. */
export function bindingFrom(accounts: AccountSummary[]): TargetBinding {
  return {
    cli: accounts.find((a) => a.is_cli_active)?.uuid ?? null,
    desktop: accounts.find((a) => a.is_desktop_active)?.uuid ?? null,
  };
}
