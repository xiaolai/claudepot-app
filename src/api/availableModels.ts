// Bindings for the Settings → "Claude Code behavior" model allowlist
// editor. Shape mirrors `commands::available_models::AvailableModelsDto`.

import { invoke } from "@tauri-apps/api/core";

export interface AvailableModelsState {
  /** The allowlist, in file order. Order is load-bearing: with
   *  `enforce`, CC's Default option resolves to the FIRST entry. */
  entries: string[];
  /** `enforceAvailableModels` as written; `null` when absent. */
  enforce: boolean | null;
  /** Whether the `availableModels` key exists at all. CC treats an
   *  absent key and `[]` as opposites: every model, or none but
   *  Default. */
  key_present: boolean;
  /** Whether any restriction is actually in force. */
  restricts_models: boolean;
  /** A present, empty list — only Default can be used. */
  blocks_all: boolean;
  /** Whether `enforce` is doing anything. CC ignores it with an empty
   *  list, and when a managed policy takes cascade trust away. */
  enforce_is_effective: boolean;
  /** `enforce` is set and a managed policy is why it does nothing. */
  enforce_overridden_by_policy: boolean;
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
