import type { CSSProperties, ReactNode } from "react";

import {
  ROTATION_OUTCOME_LABEL,
  WINDOW_LABELS,
  type RotationAuditEntry,
  type RotationRule,
} from "../../api/rotation";
import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";

/* List-row, audit-table, and empty-state pieces extracted from
 * RotationPane.tsx to keep that file under the loc-guardian
 * production-LOC limit. Each component is a thin presentational
 * unit driven by the parent's data + handlers. */

export function RuleRow({
  rule,
  onToggle,
  onEdit,
  onDelete,
}: {
  rule: RotationRule;
  onToggle: (enabled: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const triggerWindow =
    rule.trigger.window && rule.trigger.window in WINDOW_LABELS
      ? WINDOW_LABELS[rule.trigger.window as keyof typeof WINDOW_LABELS]
      : "—";
  return (
    <li
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-12)",
        padding: "var(--sp-12) var(--sp-16)",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--rad-md)",
        opacity: rule.enabled ? 1 : 0.6,
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
            marginBottom: "var(--sp-4)",
          }}
        >
          <strong style={{ fontSize: "var(--fs-sm)" }}>{rule.id}</strong>
          <Tag>{rule.mode === "auto" ? "Auto" : "Confirm"}</Tag>
          {!rule.enabled && <Tag>Disabled</Tag>}
        </div>
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
          {triggerWindow} ≥ {rule.trigger.pct}% → {selectorSummary(rule)}
        </div>
      </div>
      <Button variant="ghost" onClick={() => onToggle(!rule.enabled)}>
        {rule.enabled ? "Disable" : "Enable"}
      </Button>
      <IconButton
        glyph={NF.edit}
        title="Edit rule"
        aria-label="Edit rule"
        onClick={onEdit}
      />
      <IconButton
        glyph={NF.trash}
        title="Delete rule"
        aria-label="Delete rule"
        onClick={onDelete}
      />
    </li>
  );
}

function selectorSummary(rule: RotationRule): string {
  const sel = rule.action.selector;
  switch (sel.kind) {
    case "least_used":
      return `least-used among ${sel.candidates.length} accounts`;
    case "round_robin":
      return `round-robin across ${sel.candidates.length} accounts`;
    case "explicit":
      return `swap to ${sel.email}`;
  }
}

export function EmptyRulesPanel({ hasAccounts }: { hasAccounts: boolean }) {
  return (
    <div
      style={{
        padding: "var(--sp-20)",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--rad-md)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {hasAccounts ? (
        <>
          No rotation rules yet. Add one to auto-switch accounts when
          usage thresholds trip.
        </>
      ) : (
        <>
          Add at least one Anthropic account before creating rotation
          rules — the candidate picker reads the account list.
        </>
      )}
    </div>
  );
}

export function AuditTable({ entries }: { entries: RotationAuditEntry[] }) {
  return (
    <div
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--rad-md)",
        overflow: "hidden",
      }}
    >
      <table
        style={{
          width: "100%",
          borderCollapse: "collapse",
          fontSize: "var(--fs-xs)",
        }}
      >
        <thead style={{ background: "var(--bg-sunken)" }}>
          <tr>
            <Th>When</Th>
            <Th>Rule</Th>
            <Th>From → To</Th>
            <Th>Outcome</Th>
            <Th>Reason</Th>
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <tr
              key={e.id}
              style={{ borderTop: "var(--bw-hair) solid var(--line)" }}
            >
              <Td>{formatRelative(new Date(e.ts).getTime() / 1000)}</Td>
              <Td>{e.ruleId}</Td>
              <Td>
                {e.fromEmail}
                {e.toEmail ? ` → ${e.toEmail}` : ""}
              </Td>
              <Td>
                <Tag>{ROTATION_OUTCOME_LABEL[e.outcome] ?? e.outcome}</Tag>
              </Td>
              <Td style={{ color: "var(--fg-muted)" }}>
                {e.reason}
                {e.trigger.bgWorkers != null && e.trigger.bgWorkers > 0 && (
                  <>
                    {e.reason && " "}
                    <Tag>
                      {e.trigger.bgWorkers} bg worker
                      {e.trigger.bgWorkers === 1 ? "" : "s"} active
                    </Tag>
                  </>
                )}
              </Td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Th({ children }: { children: ReactNode }) {
  return (
    <th
      style={{
        textAlign: "left",
        padding: "var(--sp-6) var(--sp-12)",
        fontWeight: 600,
        color: "var(--fg-muted)",
      }}
    >
      {children}
    </th>
  );
}

function Td({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <td style={{ padding: "var(--sp-6) var(--sp-12)", ...style }}>{children}</td>
  );
}
