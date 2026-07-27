// Activities → Usage → "Unused" table.
//
// Separate from `UsageTable` because the columns differ: a table of
// zeroes carries no information, so the count column is replaced by
// last-modified age (dev-docs/ecosystem-pane.md §5.1).
//
// ── Why there are no Disable/Trash row actions ───────────────────────
// `artifact_lifecycle::paths::classify_path` refuses any path under the
// plugin cache (`RefuseReason::Plugin`), and the large majority of
// installed artifacts are plugin-owned. A row kebab offering Disable or
// Trash would therefore error for most rows. Per rules/design.md
// ("disabled buttons state a reason inline") the honest first cut is to
// ship the *measurement* and leave the verb out — plugin-owned
// artifacts are managed at the plugin level, not the file level.
//
// Two non-destructive row actions DO ship, because both work for every
// row regardless of ownership:
//   - Reveal in Config — deep-links to Global -> Config at this node,
//     the one surface where a plugin-owned artifact can be inspected
//     and (for standalone artifacts) disabled or trashed.
//   - Copy path (rules/path-display.md state C).

import { CopyButton } from "../../components/CopyButton";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
import type { UnusedArtifactDto } from "../../types";

export function UnusedTable({
  rows,
  onReveal,
}: {
  rows: UnusedArtifactDto[];
  /** Navigate to Global -> Config focused on this artifact's node. */
  onReveal: (row: UnusedArtifactDto) => void;
}) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-xs)",
      }}
    >
      <thead>
        <tr style={{ borderBottom: "var(--bw-hair) solid var(--line)" }}>
          <Th>Artifact</Th>
          <Th>Kind</Th>
          <Th>Plugin</Th>
          <Th align="right">Modified</Th>
          <Th align="right" srOnly>
            Actions
          </Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr
            key={`${r.kind}:${r.artifact_key}`}
            style={{ borderBottom: "var(--bw-hair) solid var(--line)" }}
          >
            <Td>
              <span className="mono selectable" title={r.abs_path}>
                {r.label}
              </span>
            </Td>
            <Td dim>{r.kind}</Td>
            <Td dim>{r.plugin_id ?? "—"}</Td>
            <Td dim align="right" nowrap>
              {formatRelative(r.modified_ms, { ago: false })}
            </Td>
            <Td align="right" nowrap>
              <IconButton
                glyph={NF.folder}
                size="sm"
                title={`Reveal ${r.label} in Config`}
                aria-label={`Reveal ${r.label} in Config`}
                onClick={() => onReveal(r)}
              />
              <CopyButton text={r.abs_path} />
            </Td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Td({
  children,
  dim = false,
  align = "left",
  nowrap = false,
}: {
  children: React.ReactNode;
  dim?: boolean;
  align?: "left" | "right";
  nowrap?: boolean;
}) {
  return (
    <td
      style={{
        padding: "var(--sp-4) var(--sp-8)",
        textAlign: align,
        ...(dim ? { color: "var(--fg-dim)" } : null),
        ...(nowrap ? { whiteSpace: "nowrap" } : null),
      }}
    >
      {children}
    </td>
  );
}

function Th({
  children,
  align = "left",
  srOnly = false,
}: {
  children: React.ReactNode;
  align?: "left" | "right";
  srOnly?: boolean;
}) {
  return (
    <th
      scope="col"
      style={{
        textAlign: align,
        padding: "var(--sp-4) var(--sp-8)",
        fontWeight: 400,
        color: "var(--fg-dim)",
        ...(srOnly
          ? {
              position: "absolute",
              width: 1,
              height: 1,
              overflow: "hidden",
              clip: "rect(0 0 0 0)",
              whiteSpace: "nowrap",
            }
          : null),
      }}
    >
      {children}
    </th>
  );
}
