// Sortable per-artifact usage table — extracted from UsageView so the
// container stays under the 350 LOC loc-guardian limit. The state-
// owning UsageView passes filtered rows + sort state and gets back
// `onSort` callbacks; the table itself is pure presentation.

import { Table, Th, ThSort, Tr, Td } from "../../components/primitives";
import { formatRelative } from "../../lib/formatRelative";
import type { ArtifactUsageRowDto } from "../../types";

export type SortKey =
  | "count_30d"
  | "count_7d"
  | "count_24h"
  | "last_seen"
  | "errors"
  | "p50";

export function UsageTable({
  rows,
  sortKey,
  onSort,
}: {
  rows: ArtifactUsageRowDto[];
  sortKey: SortKey;
  onSort: (k: SortKey) => void;
}) {
  return (
    <Table style={{ fontSize: "var(--fs-xs)" }}>
      <thead>
        <tr style={{ background: "var(--bg-sunken)" }}>
          <Th>Kind</Th>
          <Th>Artifact</Th>
          <Th>Plugin</Th>
          <ThSort current={sortKey} value="count_24h" onSort={onSort} align="right">
            24h
          </ThSort>
          <ThSort current={sortKey} value="count_7d" onSort={onSort} align="right">
            7d
          </ThSort>
          <ThSort current={sortKey} value="count_30d" onSort={onSort} align="right">
            30d
          </ThSort>
          <ThSort current={sortKey} value="last_seen" onSort={onSort} align="right">
            Last seen
          </ThSort>
          <ThSort current={sortKey} value="errors" onSort={onSort} align="right">
            Errors
          </ThSort>
          <ThSort current={sortKey} value="p50" onSort={onSort} align="right">
            p50
          </ThSort>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Row key={`${r.kind}|${r.artifact_key}`} row={r} />
        ))}
      </tbody>
    </Table>
  );
}

function Row({ row }: { row: ArtifactUsageRowDto }) {
  const s = row.stats;
  const errRate =
    s.count_30d > 0 ? (s.error_count_30d / s.count_30d) * 100 : 0;
  const errTone =
    s.error_count_30d === 0
      ? "var(--fg-faint)"
      : errRate >= 10
        ? "var(--danger)"
        : "var(--warn)";
  return (
    <Tr>
      <Td muted>{row.kind}</Td>
      <Td>
        <span
          style={{ fontWeight: 500, color: "var(--fg)" }}
          title={row.artifact_key}
        >
          {displayName(row.kind, row.artifact_key)}
        </span>
      </Td>
      <Td muted>{row.plugin_id ?? "—"}</Td>
      <Td num>{s.count_24h || "—"}</Td>
      <Td num>{s.count_7d || "—"}</Td>
      <Td num emphasis>
        {s.count_30d}
      </Td>
      <Td muted num>
        {s.last_seen_ms ? formatRelative(s.last_seen_ms, { ago: false }) : "—"}
      </Td>
      <Td num style={{ color: errTone }}>
        {s.error_count_30d || "—"}
      </Td>
      <Td muted num>
        {s.p50_ms_24h ?? s.avg_ms_30d ?? "—"}
        {(s.p50_ms_24h ?? s.avg_ms_30d) != null ? "ms" : ""}
      </Td>
    </Tr>
  );
}

/**
 * Strip the canonical prefix so the UI shows the artifact's natural
 * name. Mirrors the inverse of `extract_helpers::parse_*` —
 * displayName(kind, key) ⊆ extractor(kind, name) inverse.
 *
 *   skill: plugin:foo:bar    → bar
 *   skill: userSettings:bar  → bar
 *   skill: projectSettings:bar → bar
 *   agent: foo:bar           → bar
 *   agent: bar               → bar
 *   command: /foo:bar        → /foo:bar  (the slash form IS the name)
 */
export function displayName(kind: string, key: string): string {
  if (kind === "skill") {
    if (key.startsWith("plugin:")) {
      const parts = key.split(":");
      return parts.slice(2).join(":");
    }
    if (key.startsWith("userSettings:")) return key.slice("userSettings:".length);
    if (key.startsWith("projectSettings:"))
      return key.slice("projectSettings:".length);
  }
  if (kind === "agent") {
    const i = key.indexOf(":");
    return i > 0 ? key.slice(i + 1) : key;
  }
  return key;
}

