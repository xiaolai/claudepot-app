// Bindings for the Settings → "Claude Code behavior" extended-thinking
// toggle. Shape mirrors `claudepot_core::thinking_toggle::ThinkingState`
// (returned verbatim by the `thinking_*` commands — no DTO).

import { invoke } from "@tauri-apps/api/core";

export type ThinkingDecisionSource =
  | "env_max_thinking_tokens"
  | "user_settings"
  | "default";

export interface ThinkingState {
  /** Whether extended thinking is on by default for new sessions. */
  effective: boolean;
  decided_by: ThinkingDecisionSource;
  /** `false` when MAX_THINKING_TOKENS forces the decision (toggle locked). */
  user_writable: boolean;
  user_settings_value: boolean | null;
  env_max_thinking_tokens_set: boolean;
}

export const thinkingApi = {
  thinkingState: () => invoke<ThinkingState>("thinking_state"),

  /** `enabled = true` returns to CC's default (key cleared). */
  thinkingSet: (enabled: boolean) =>
    invoke<ThinkingState>("thinking_set", { enabled }),
};
