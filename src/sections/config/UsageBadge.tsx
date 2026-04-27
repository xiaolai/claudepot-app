// Two flavors of usage indicator for Config artifacts:
//
//   <UsageMicroBadge>  — tiny inline `· 12` annotation for the dense
//                        tree row. Renders nothing at count===0
//                        (the row dim treatment carries that signal).
//   <UsageStrip>       — full one-line strip for the preview pane:
//                        "12 fires · last 2h · 0 errors · p50 28ms".
//
// Both accept the same `ArtifactUsageStatsDto`. They render nothing
// when given `null` (haven't fetched yet) — the calling component is
// responsible for distinguishing "loading" from "never used".

import { formatRelative } from "../../lib/formatRelative";
import type { ArtifactUsageStatsDto } from "../../types";

export function UsageMicroBadge({
  stats,
}: {
  stats: ArtifactUsageStatsDto | null | undefined;
}) {
  if (!stats || stats.count_30d === 0) return null;
  const errBadge = stats.error_count_30d > 0;
  return (
    <span
      aria-label={`${stats.count_30d} invocations in last 30 days${errBadge ? `, ${stats.error_count_30d} errors` : ""}`}
      title={
        errBadge
          ? `${stats.count_30d} fires · ${stats.error_count_30d} errors (30d)`
          : `${stats.count_30d} fires (30d)`
      }
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-2)",
        marginLeft: "var(--sp-6)",
        fontSize: "var(--fs-2xs)",
        color: errBadge ? "var(--warn)" : "var(--fg-faint)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      ·{" "}
      <span style={{ fontWeight: 500 }}>{formatCount(stats.count_30d)}</span>
      {errBadge && <span aria-hidden="true">!</span>}
    </span>
  );
}

export function UsageStrip({
  stats,
}: {
  stats: ArtifactUsageStatsDto | null | undefined;
}) {
  if (!stats) return null;
  if (stats.count_30d === 0) {
    return (
      <div
        role="note"
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          padding: "var(--sp-6) var(--sp-20)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        Never invoked in the last 30 days
      </div>
    );
  }
  const parts: string[] = [];
  parts.push(`${stats.count_30d} ${stats.count_30d === 1 ? "fire" : "fires"} (30d)`);
  if (stats.count_7d > 0 && stats.count_7d !== stats.count_30d) {
    parts.push(`${stats.count_7d} (7d)`);
  }
  if (stats.count_24h > 0 && stats.count_24h !== stats.count_7d) {
    parts.push(`${stats.count_24h} (24h)`);
  }
  if (stats.last_seen_ms != null) {
    parts.push(`last ${formatRelative(stats.last_seen_ms, { ago: true })}`);
  }
  if (stats.error_count_30d > 0) {
    parts.push(`${stats.error_count_30d} errors`);
  }
  if (stats.p50_ms_24h != null) {
    parts.push(`p50 ${stats.p50_ms_24h}ms`);
  } else if (stats.avg_ms_30d != null) {
    parts.push(`avg ${stats.avg_ms_30d}ms`);
  }
  const errBadge = stats.error_count_30d > 0;
  return (
    <div
      role="note"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-6) var(--sp-20)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        fontSize: "var(--fs-2xs)",
        color: errBadge ? "var(--warn)" : "var(--fg-muted)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      <span aria-hidden="true">↳</span>
      <span>{parts.join(" · ")}</span>
    </div>
  );
}

function formatCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10000) return `${(n / 1000).toFixed(1)}k`;
  return `${Math.round(n / 1000)}k`;
}

