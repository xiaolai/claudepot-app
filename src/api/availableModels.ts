// Bindings for the Settings → "Claude Code behavior" model allowlist
// editor. Shape mirrors `commands::available_models::AvailableModelsDto`.

import { invoke } from "@tauri-apps/api/core";

export interface AvailableModelsState {
  /** The allowlist, in file order. Order is load-bearing: with
   *  `enforce`, CC's Default option resolves to the FIRST entry. */
  entries: string[];
  /** `enforceAvailableModels` as written; `null` when absent. */
  enforce: boolean | null;
  /** Whether the `availableModels` key exists at all — distinguishes
   *  "no allowlist" from "an explicitly empty one". */
  key_present: boolean;
  /** Whether any restriction is actually in force. */
  restricts_models: boolean;
  /** Whether `enforce` is doing anything. CC ignores it with an empty
   *  list, so a `true` with no entries is inert. */
  enforce_is_effective: boolean;
  /** Minimum CC version that honors the enforce flag. */
  enforce_min_cc_version: string;
}

export const availableModelsApi = {
  availableModelsState: () =>
    invoke<AvailableModelsState>("available_models_state"),

  /** Replaces both keys in one atomic write. Returns the re-resolved
   *  state, whose `entries` reflect the backend's normalization. */
  availableModelsSet: (entries: string[], enforce: boolean) =>
    invoke<AvailableModelsState>("available_models_set", { entries, enforce }),
};
