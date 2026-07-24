// Bindings for the Global → Memory "Background memory consolidation"
// card. Shape mirrors `claudepot_core::auto_dream::AutoDreamState`
// (returned verbatim by the `auto_dream_*` commands — no DTO).

import { invoke } from "@tauri-apps/api/core";

/** Default = key absent (CC's server flag decides); On/Off = explicit. */
export type AutoDreamMode = "default" | "on" | "off";

export interface AutoDreamState {
  mode: AutoDreamMode;
  user_settings_value: boolean | null;
  /** Consolidation can't run while auto-memory is off — the card
   *  disables editing and states the dependency when this is false. */
  auto_memory_enabled: boolean;
}

export const autoDreamApi = {
  autoDreamState: () => invoke<AutoDreamState>("auto_dream_state"),

  autoDreamSet: (mode: AutoDreamMode) =>
    invoke<AutoDreamState>("auto_dream_set", { mode }),
};
