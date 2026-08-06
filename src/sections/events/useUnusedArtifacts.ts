// Activities → Usage → "Unused": installed artifacts with no recorded
// invocation.
//
// This hook is a thin fetch wrapper. **All business logic lives in
// `claudepot_core::artifact_usage::unused`** — identity derivation,
// dedup across cached plugin versions, ledger subtraction, the grace
// window, and the disabled-plugin exclusion. Per
// `.claude/rules/architecture.md` those are business rules, not
// rendering, and an earlier revision of this file wrongly owned them.
//
// ── What the rows can and cannot claim ───────────────────────────────
// The ledger records "ever observed **by Claudepot**". Invocations that
// predate `artifact_first_last` (schema v6), or that lived only in
// transcripts deleted before its backfill re-scan, are absent. So a row
// means "no invocation on record" — NOT "never used". Every string the
// UI renders must respect that.
//
// Bundled MCP servers are not covered: `ArtifactKind` spans
// skill/hook/agent/command, while MCP calls land in `tool_calls`. This
// is why the view answers at artifact granularity and must never be
// rolled up into "this plugin is unused".

import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import type { UnusedArtifactDto, UnusedReportDto } from "../../types";

export interface UnusedArtifactsResult {
  rows: UnusedArtifactDto[];
  /** Distinct installed artifacts considered, after dedup. */
  installedCount: number;
  /** Held back by the grace window. */
  suppressedRecent: number;
  /** Excluded because their owning plugin is not enabled. */
  suppressedDisabled: number;
  /** Grace window in days, as core applied it. */
  graceDays: number;
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

const EMPTY: UnusedReportDto = {
  rows: [],
  installed_count: 0,
  suppressed_recent: 0,
  suppressed_disabled: 0,
  grace_days: 0,
};

export function useUnusedArtifacts(enabled: boolean): UnusedArtifactsResult {
  const [report, setReport] = useState<UnusedReportDto>(EMPTY);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const refresh = useCallback(() => setTick((n) => n + 1), []);

  useEffect(() => {
    // Only pay for this while the chip is active — the backend walks
    // the filesystem, far heavier than the shared usage rollup the
    // other three views poll.
    if (!enabled) return;
    let cancelled = false;
    setLoading(true);
    setError(null);

    void (async () => {
      try {
        const r = await api.artifactUsageUnused();
        if (!cancelled) setReport(r);
      } catch (e) {
        // Fail closed: surface the error rather than rendering an empty
        // list, which would read as "nothing is unused" — a confident
        // wrong answer instead of an honest failure.
        if (!cancelled) setError(renderError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [enabled, tick]);

  return {
    rows: report.rows,
    installedCount: report.installed_count,
    suppressedRecent: report.suppressed_recent,
    suppressedDisabled: report.suppressed_disabled,
    graceDays: report.grace_days,
    loading,
    error,
    refresh,
  };
}
