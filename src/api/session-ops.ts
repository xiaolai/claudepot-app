// Session prune / slim / trash / export-preview / share + GitHub token.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  PruneFilterInput,
  PrunePlan,
  SlimOptsInput,
  SlimPlan,
  BulkSlimPlan,
  TrashListing,
  ExportFormatInput,
  RedactionPolicyInput,
  GithubTokenStatus,
} from "../types";

export const sessionOpsApi = {
  // ---------- Prune / slim / trash ----------
  /** Preview which sessions match a prune filter. Pure — no disk writes. */
  sessionPrunePlan: (filter: PruneFilterInput) =>
    invoke<PrunePlan>("session_prune_plan", { filter }),
  /** Start an async prune op; returns an op_id to subscribe on. */
  sessionPruneStart: (filter: PruneFilterInput) =>
    invoke<string>("session_prune_start", { filter }),
  /** Preview a slim rewrite without touching disk. */
  sessionSlimPlan: (path: string, opts: SlimOptsInput) =>
    invoke<SlimPlan>("session_slim_plan", { path, opts }),
  /** Start an async slim op; returns an op_id to subscribe on. */
  sessionSlimStart: (path: string, opts: SlimOptsInput) =>
    invoke<string>("session_slim_start", { path, opts }),
  /** Preview a bulk slim across every session matching the filter. */
  sessionSlimPlanAll: (filter: PruneFilterInput, opts: SlimOptsInput) =>
    invoke<BulkSlimPlan>("session_slim_plan_all", { filter, opts }),
  /** Start an async bulk slim op; returns an op_id to subscribe on. */
  sessionSlimStartAll: (filter: PruneFilterInput, opts: SlimOptsInput) =>
    invoke<string>("session_slim_start_all", { filter, opts }),
  /** List trash batches. */
  sessionTrashList: (olderThanSecs?: number) =>
    invoke<TrashListing>("session_trash_list", {
      olderThanSecs: olderThanSecs ?? null,
    }),
  /** Restore a trashed batch to its original cwd or an override. */
  sessionTrashRestore: (entryId: string, overrideCwd?: string) =>
    invoke<string>("session_trash_restore", {
      entryId,
      overrideCwd: overrideCwd ?? null,
    }),
  /** Permanently delete trash batches. Returns bytes freed. */
  sessionTrashEmpty: (olderThanSecs?: number) =>
    invoke<number>("session_trash_empty", {
      olderThanSecs: olderThanSecs ?? null,
    }),

  // ---------- Export preview / share (Phase 3) ----------
  /** Pure preview — identical to what a file export would write. */
  sessionExportPreview: (
    target: string,
    format: ExportFormatInput,
    policy?: RedactionPolicyInput,
  ) =>
    invoke<string>("session_export_preview", {
      target,
      format,
      policy: policy ?? null,
    }),
  /** Start a gist upload; returns an op_id. */
  sessionShareGistStart: (
    target: string,
    format: ExportFormatInput,
    policy: RedactionPolicyInput | undefined,
    isPublic: boolean,
  ) =>
    invoke<string>("session_share_gist_start", {
      target,
      format,
      policy: policy ?? null,
      public: isPublic,
    }),
  /** GitHub PAT management — last4 is the only value that ever crosses. */
  settingsGithubTokenGet: () =>
    invoke<GithubTokenStatus>("settings_github_token_get"),
  settingsGithubTokenSet: (value: string) =>
    invoke<GithubTokenStatus>("settings_github_token_set", { value }),
  settingsGithubTokenClear: () => invoke<void>("settings_github_token_clear"),

};
