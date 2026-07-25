// Bindings for the Settings → "Claude Code behavior" fast-mode toggle.
// Shape mirrors `commands::fast_mode_toggle::FastModeStateDto`, which
// flattens `claudepot_core::fast_mode_toggle::FastModeState` and adds
// the rate/model facts the copy needs.

import { invoke } from "@tauri-apps/api/core";

export type FastModeDecisionSource =
  | "env_disabled"
  | "user_settings"
  | "default";

/** Rate and model facts, sourced from core so the UI copy can't drift
 *  from the rate table. */
export interface FastModeFacts {
  /** Models fast mode runs on. */
  models: string[];
  input_per_mtok: number;
  output_per_mtok: number;
}

export interface FastModeState {
  /** Whether fast mode is on by default for new sessions. */
  effective: boolean;
  decided_by: FastModeDecisionSource;
  /** `false` when CLAUDE_CODE_DISABLE_FAST_MODE forces the decision. */
  user_writable: boolean;
  user_settings_value: boolean | null;
  /** When true, every session starts with fast mode off regardless. */
  per_session_opt_in: boolean;
  env_disabled: boolean;
  facts: FastModeFacts;
}

export const fastModeApi = {
  fastModeState: () => invoke<FastModeState>("fast_mode_state"),

  /** `enabled = false` returns to CC's default (key cleared). */
  fastModeSet: (enabled: boolean) =>
    invoke<FastModeState>("fast_mode_set", { enabled }),

  fastModeSetPerSession: (required: boolean) =>
    invoke<FastModeState>("fast_mode_set_per_session", { required }),
};
