// Bindings for the Settings → General "Keep companion output local"
// toggle. Shape mirrors `claudepot_core::artifact_toggle::ArtifactState`
// (returned verbatim by the `artifact_tool_*` commands — no DTO).

import { invoke } from "@tauri-apps/api/core";

export type ArtifactDecisionSource =
  | "env_disable"
  | "disable_setting"
  | "enable_setting"
  | "default";

export interface ArtifactState {
  /** Whether CC's Artifact (cloud-publish) tool is enabled. */
  enabled: boolean;
  decided_by: ArtifactDecisionSource;
  /** `false` when the env var is forcing the decision (toggle locked). */
  user_writable: boolean;
  user_enable_value: boolean | null;
  user_disable_value: boolean | null;
  env_disable_set: boolean;
}

export const artifactToolApi = {
  artifactToolState: () => invoke<ArtifactState>("artifact_tool_state"),

  /** `enabled = false` keeps companion output local. */
  artifactToolSet: (enabled: boolean) =>
    invoke<ArtifactState>("artifact_tool_set", { enabled }),
};
