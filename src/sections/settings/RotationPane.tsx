import { useCallback, useEffect, useMemo, useState } from "react";

import { api } from "../../api";
import {
  ROTATION_OUTCOME_LABEL,
  WINDOW_LABELS,
  type RotationAuditEntry,
  type RotationRule,
  type RotationRulesFile,
} from "../../api/rotation";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { formatRelative } from "../../lib/formatRelative";
import type { AccountSummary } from "../../types";
import { RotationRuleModal } from "./RotationRuleModal";

interface Props {
  pushToast: (kind: "info" | "error", text: string) => void;
}

/**
 * Settings → Rotation pane. Manage user-authored rules that swap
 * the CLI-active account when an Anthropic-reported usage window
 * crosses a threshold. See `dev-docs/auto-rotation.md`.
 */
export function RotationPane({ pushToast }: Props) {
  const [file, setFile] = useState<RotationRulesFile | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [audit, setAudit] = useState<RotationAuditEntry[]>([]);
  const [editingRule, setEditingRule] = useState<RotationRule | null>(null);
  const [adding, setAdding] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [f, accts, a] = await Promise.all([
        api.rotationRulesGet(),
        api.accountList(),
        api.rotationAuditGet(20),
      ]);
      setFile(f);
      setAccounts(accts);
      setAudit(a);
    } catch (e) {
      pushToast("error", `Rotation load failed: ${e}`);
    }
  }, [pushToast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const candidateEmails = useMemo(
    () => accounts.map((a) => a.email).sort(),
    [accounts],
  );

  const saveRules = useCallback(
    async (next: RotationRulesFile) => {
      try {
        await api.rotationRulesSet(next);
        setFile(next);
        // Refresh audit too — the rule add itself doesn't change it,
        // but a follow-up tick might already have emitted a suggestion.
        const a = await api.rotationAuditGet(20);
        setAudit(a);
      } catch (e) {
        pushToast("error", `Rotation save failed: ${e}`);
        throw e;
      }
    },
    [pushToast],
  );

  const handleSaveRule = useCallback(
    async (rule: RotationRule, original: RotationRule | null) => {
      if (!file) return;
      const idx = original
        ? file.rules.findIndex((r) => r.id === original.id)
        : -1;
      const nextRules = idx >= 0 ? [...file.rules] : [...file.rules, rule];
      if (idx >= 0) nextRules[idx] = rule;
      const next: RotationRulesFile = { ...file, rules: nextRules };
      await saveRules(next);
      setEditingRule(null);
      setAdding(false);
      pushToast("info", `Rotation rule "${rule.id}" saved`);
    },
    [file, pushToast, saveRules],
  );

  const handleToggleEnabled = useCallback(
    async (id: string, enabled: boolean) => {
      if (!file) return;
      const nextRules = file.rules.map((r) =>
        r.id === id ? { ...r, enabled } : r,
      );
      await saveRules({ ...file, rules: nextRules });
    },
    [file, saveRules],
  );

  const handleDelete = useCallback(
    async (id: string) => {
      if (!file) return;
      const nextRules = file.rules.filter((r) => r.id !== id);
      await saveRules({ ...file, rules: nextRules });
      pushToast("info", `Rotation rule "${id}" deleted`);
    },
    [file, pushToast, saveRules],
  );

  if (!file) {
    return <div style={{ color: "var(--fg-faint)" }}>Loading rotation rules…</div>;
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-24)",
      }}
    >
      <p
        style={{
          margin: 0,
          color: "var(--fg-muted)",
          fontSize: "var(--fs-sm)",
          maxWidth: "var(--content-cap-md, tokens.content.cap.md)",
        }}
      >
        Auto-switch the active CLI account when an Anthropic usage
        window crosses a threshold you set. Triggers fire from
        Anthropic's <code>/api/oauth/usage</code> data, polled every 5
        minutes. The default <strong>Confirm</strong> mode shows a
        toast and waits for your click; switch to <strong>Auto</strong>
        once you trust the rule.
      </p>

      <section>
        <header
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: "var(--sp-12)",
          }}
        >
          <h3
            style={{
              margin: 0,
              fontSize: "var(--fs-md)",
              fontWeight: 600,
              letterSpacing: "var(--ls-tight)",
            }}
          >
            Rules
          </h3>
          <Button
            glyph={NF.plus}
            variant="solid"
            onClick={() => setAdding(true)}
            disabled={candidateEmails.length === 0}
          >
            Add rule
          </Button>
        </header>

        {file.rules.length === 0 ? (
          <EmptyRulesPanel hasAccounts={candidateEmails.length > 0} />
        ) : (
          <ul
            style={{
              listStyle: "none",
              margin: 0,
              padding: 0,
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-8)",
            }}
          >
            {file.rules.map((rule) => (
              <RuleRow
                key={rule.id}
                rule={rule}
                onToggle={(en) => handleToggleEnabled(rule.id, en)}
                onEdit={() => setEditingRule(rule)}
                onDelete={() => setConfirmDeleteId(rule.id)}
              />
            ))}
          </ul>
        )}
      </section>

      <section>
        <h3
          style={{
            margin: 0,
            marginBottom: "var(--sp-12)",
            fontSize: "var(--fs-md)",
            fontWeight: 600,
            letterSpacing: "var(--ls-tight)",
          }}
        >
          Recent activity
        </h3>
        {audit.length === 0 ? (
          <p style={{ margin: 0, color: "var(--fg-faint)", fontSize: "var(--fs-sm)" }}>
            No rotation activity yet.
          </p>
        ) : (
          <AuditTable entries={audit} />
        )}
      </section>

      {(adding || editingRule) && (
        <RotationRuleModal
          existing={editingRule}
          existingIds={file.rules.map((r) => r.id)}
          candidateEmails={candidateEmails}
          onClose={() => {
            setAdding(false);
            setEditingRule(null);
          }}
          onSave={(rule: RotationRule) => handleSaveRule(rule, editingRule)}
        />
      )}

      {confirmDeleteId && (
        <ConfirmDialog
          title="Delete rotation rule?"
          body={
            <>
              Rule <code>{confirmDeleteId}</code> will be removed.
              You can re-add it later.
            </>
          }
          confirmLabel="Delete"
          confirmDanger
          onCancel={() => setConfirmDeleteId(null)}
          onConfirm={() => {
            const id = confirmDeleteId;
            setConfirmDeleteId(null);
            void handleDelete(id);
          }}
        />
      )}
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */

function RuleRow({
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
      <Button
        variant="ghost"
        onClick={() => onToggle(!rule.enabled)}
      >
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

function EmptyRulesPanel({ hasAccounts }: { hasAccounts: boolean }) {
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

function AuditTable({ entries }: { entries: RotationAuditEntry[] }) {
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
            <tr key={e.id} style={{ borderTop: "var(--bw-hair) solid var(--line)" }}>
              <Td>{formatRelative(new Date(e.ts).getTime() / 1000)}</Td>
              <Td>{e.ruleId}</Td>
              <Td>
                {e.fromEmail}
                {e.toEmail ? ` → ${e.toEmail}` : ""}
              </Td>
              <Td>
                <Tag>{ROTATION_OUTCOME_LABEL[e.outcome] ?? e.outcome}</Tag>
              </Td>
              <Td style={{ color: "var(--fg-muted)" }}>{e.reason}</Td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Th({ children }: { children: React.ReactNode }) {
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

function Td({
  children,
  style,
}: {
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <td
      style={{
        padding: "var(--sp-6) var(--sp-12)",
        ...style,
      }}
    >
      {children}
    </td>
  );
}

