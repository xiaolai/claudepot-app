// Bindings for the Settings → "Claude Code behavior" commit/PR
// attribution control. Shape mirrors
// `claudepot_core::attribution_settings::AttributionState` (returned
// verbatim by the `attribution_*` commands — no DTO).

import { invoke } from "@tauri-apps/api/core";

/** Default = CC's trailer; Off = suppressed; Custom = user text. */
export type AttributionModeKind = "default" | "off" | "custom";

export interface AttributionState {
  mode: AttributionModeKind;
  /** `attribution.commit`, if present (prefills the Custom editor). */
  commit: string | null;
  /** `attribution.pr`, if present. */
  pr: string | null;
  include_co_authored_by: boolean | null;
}

export const attributionApi = {
  attributionState: () => invoke<AttributionState>("attribution_state"),

  /**
   * `mode = "custom"` writes the literal `commit` / `pr` strings.
   * For `"default"` / `"off"`, the text args are ignored.
   */
  attributionSet: (
    mode: AttributionModeKind,
    commit?: string,
    pr?: string,
  ) =>
    invoke<AttributionState>("attribution_set", {
      mode,
      commit: commit ?? null,
      pr: pr ?? null,
    }),
};
