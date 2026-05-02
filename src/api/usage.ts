// Local cost report — frontend binding for the
// `local_usage_aggregate` Tauri command. The command lives in
// `src-tauri/src/commands_usage_local.rs`; this slice converts the
// idiomatic-TS window shape (`{ kind: "lastDays", days }`) to the
// idiomatic-Rust wire shape (`{ kind: "last_days", days }`) so
// neither side has to compromise its naming.

import { invoke } from "@tauri-apps/api/core";
import type {
  LocalUsageReport,
  TopCostlyPrompts,
  UsageWindowSpec,
} from "../types";

interface WireWindow {
  kind: string;
  days?: number;
}

function toWire(spec: UsageWindowSpec): WireWindow {
  if (spec.kind === "all") {
    return { kind: "all" };
  }
  return { kind: "last_days", days: spec.days };
}

export const usageApi = {
  /**
   * Aggregate local cost + token totals from on-disk transcripts.
   *
   * Pure read — never blocks the UI for more than a single
   * `list_all_sessions` refresh pass against `~/.claudepot/sessions.db`.
   * Returns a fully-resolved `LocalUsageReport` including a
   * `pricing_source` pill and an optional `pricing_error` so the
   * caller can render the trust signal alongside the numbers.
   */
  localUsageAggregate: (window: UsageWindowSpec) =>
    invoke<LocalUsageReport>("local_usage_aggregate", {
      spec: toWire(window),
    }),
  /**
   * Install-wide top-N costliest prompts for the supplied window,
   * scored against the user's active pricing tier. The backend caps
   * `final_n` at 50 server-side; passing a larger value is silently
   * truncated. Returns `{ turns: [], pricing_tier }` when no turns
   * have been indexed yet (fresh install with sessions on disk but
   * no re-scan to populate the per-turn table).
   */
  topCostlyPrompts: (window: UsageWindowSpec, finalN: number) =>
    invoke<TopCostlyPrompts>("top_costly_prompts", {
      spec: toWire(window),
      finalN,
    }),
};
