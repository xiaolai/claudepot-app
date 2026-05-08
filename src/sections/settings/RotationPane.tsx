import { useCallback, useEffect, useMemo, useState } from "react";

import { api } from "../../api";
import {
  type RotationAuditEntry,
  type RotationRule,
  type RotationRulesFile,
} from "../../api/rotation";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Button } from "../../components/primitives/Button";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import {
  AuditTable,
  EmptyRulesPanel,
  RuleRow,
} from "./RotationPane.bits";
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
          maxWidth: "var(--content-cap-md)",
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

