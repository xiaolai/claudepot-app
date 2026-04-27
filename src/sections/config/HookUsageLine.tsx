// One-line usage strip rendered under each hook command in
// HooksRenderer. Sharded out so HooksRenderer.tsx stays under the
// loc-guardian limit, and so the canonical `hookArtifactKey` mirror
// of the Rust helper has a single home.

import { formatRelative } from "../../lib/formatRelative";
import type { ArtifactUsageStatsDto } from "../../types";

/**
 * Reads the same artifact_key the JSONL extractor wrote
 * (`<hookName>|<command>`) so the data is 1:1 with what CC executed.
 *
 * Renders a stable-height transparent placeholder while the batch
 * fetch is in flight (parent guards with empty-Map default), so the
 * card layout doesn't jump when results arrive. When stats arrive
 * but count_30d is 0 the line says "never fired" — important for
 * hooks specifically because hooks fire silently and a misconfigured
 * one looks identical to a working one without this signal.
 */
export function HookUsageLine({
  stats,
}: {
  stats: ArtifactUsageStatsDto | undefined;
}) {
  if (!stats) {
    return (
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "transparent",
          height: "var(--lh-2xs)",
        }}
        aria-hidden
      >
        ·
      </div>
    );
  }
  if (stats.count_30d === 0) {
    return (
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        Never fired in last 30 days
      </div>
    );
  }
  const errs = stats.error_count_30d;
  const tone = errs > 0 ? "var(--warn)" : "var(--fg-muted)";
  const parts: string[] = [];
  parts.push(`${stats.count_30d} ${stats.count_30d === 1 ? "fire" : "fires"} (30d)`);
  if (stats.count_24h > 0 && stats.count_24h !== stats.count_30d) {
    parts.push(`${stats.count_24h} (24h)`);
  }
  if (stats.last_seen_ms != null) {
    parts.push(`last ${formatRelative(stats.last_seen_ms, { ago: true })}`);
  }
  if (errs > 0) parts.push(`${errs} errors`);
  if (stats.p50_ms_24h != null) {
    parts.push(`p50 ${stats.p50_ms_24h}ms`);
  } else if (stats.avg_ms_30d != null) {
    parts.push(`avg ${stats.avg_ms_30d}ms`);
  }
  return (
    <div
      style={{
        fontSize: "var(--fs-2xs)",
        color: tone,
        fontVariantNumeric: "tabular-nums",
      }}
    >
      ↳ {parts.join(" · ")}
    </div>
  );
}

/**
 * Mirror of `claudepot_core::artifact_usage::extract_helpers::hook_artifact_key`.
 * Two hooks sharing a shell command but firing on different events
 * are distinct artifacts; the key combines `<hookName>|<command>`
 * so renderer joins line up with extractor writes. Keep in lockstep
 * with the Rust helper.
 */
export function hookArtifactKey(
  hookName: string | null | undefined,
  command: string | null | undefined,
): string | null {
  const n = hookName && hookName.length > 0 ? hookName : null;
  const c = command && command.length > 0 ? command : null;
  if (n && c) return `${n}|${c}`;
  if (n) return n;
  if (c) return c;
  return null;
}
