import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { Tag } from "../components/primitives/Tag";
import { ToastContainer } from "../components/ToastContainer";
import { useToasts } from "../hooks/useToasts";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import type {
  AccountSummary,
  ApiKeySummary,
  OauthTokenSummary,
} from "../types";
import { AddKeyModal } from "./keys/AddKeyModal";
import { OAuthUsageModal } from "./keys/OAuthUsageModal";

type PendingRemoval =
  | { kind: "api"; row: ApiKeySummary }
  | { kind: "oauth"; row: OauthTokenSummary };

export function KeysSection() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const [apiKeys, setApiKeys] = useState<ApiKeySummary[]>([]);
  const [oauthTokens, setOauthTokens] = useState<OauthTokenSummary[]>([]);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [usageModalFor, setUsageModalFor] = useState<OauthTokenSummary | null>(
    null,
  );
  const [pendingRemoval, setPendingRemoval] = useState<PendingRemoval | null>(
    null,
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [api_, oauth, accts] = await Promise.all([
        api.keyApiList(),
        api.keyOauthList(),
        api.accountList(),
      ]);
      setApiKeys(api_);
      setOauthTokens(oauth);
      setAccounts(accts);
    } catch (e) {
      pushToast("error", `Load failed: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onCopy = useCallback(
    async (
      kind: "api" | "oauth",
      uuid: string,
      label: string,
      preview: string,
    ) => {
      try {
        const token =
          kind === "api"
            ? await api.keyApiCopy(uuid)
            : await api.keyOauthCopy(uuid);
        await navigator.clipboard.writeText(token);
        pushToast(
          "info",
          `Copied ${label} (${preview}) — clipboard clears in 30s.`,
        );
        scheduleClipboardClear(token);
      } catch (e) {
        pushToast("error", `Copy failed: ${e}`);
      }
    },
    [pushToast],
  );

  const confirmRemoval = useCallback(async () => {
    if (!pendingRemoval) return;
    const { kind, row } = pendingRemoval;
    try {
      if (kind === "api") await api.keyApiRemove(row.uuid);
      else await api.keyOauthRemove(row.uuid);
      pushToast("info", `Removed ${row.label}.`);
      await refresh();
    } catch (e) {
      pushToast("error", `Remove failed: ${e}`);
    } finally {
      setPendingRemoval(null);
    }
  }, [pendingRemoval, pushToast, refresh]);

  const onProbe = useCallback(
    async (row: OauthTokenSummary) => {
      try {
        const fresh = await api.keyOauthProbe(row.uuid);
        setOauthTokens((prev) =>
          prev.map((r) => (r.uuid === fresh.uuid ? fresh : r)),
        );
        pushToast(
          fresh.last_probe_status === "ok" ? "info" : "error",
          `${row.label}: ${describeProbe(fresh.last_probe_status)}`,
        );
      } catch (e) {
        pushToast("error", `Probe failed: ${e}`);
      }
    },
    [pushToast],
  );

  const added = useCallback(
    (kind: "api" | "oauth") => {
      pushToast(
        "info",
        kind === "api" ? "API key added." : "OAuth token added.",
      );
      setAdding(false);
      void refresh();
    },
    [pushToast, refresh],
  );

  return (
    <>
      <ScreenHeader
        title="Keys"
        subtitle="Anthropic API keys and Claude Code OAuth tokens."
        actions={
          <Button
            variant="solid"
            glyph={NF.plus}
            onClick={() => setAdding(true)}
          >
            Add key
          </Button>
        }
      />

      <main
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-24) var(--sp-32) var(--sp-40)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-32)",
        }}
      >
        <ApiKeysTable
          rows={apiKeys}
          loading={loading}
          onCopy={(row) =>
            void onCopy("api", row.uuid, row.label, row.token_preview)
          }
          onRemove={(row) => setPendingRemoval({ kind: "api", row })}
        />

        <OauthTokensTable
          rows={oauthTokens}
          loading={loading}
          onCopy={(row) =>
            void onCopy("oauth", row.uuid, row.label, row.token_preview)
          }
          onRemove={(row) => setPendingRemoval({ kind: "oauth", row })}
          onProbe={onProbe}
          onOpenUsage={setUsageModalFor}
        />
      </main>

      {pendingRemoval && (
        <ConfirmDialog
          title="Remove key?"
          body={
            <p style={{ margin: 0, lineHeight: 1.5 }}>
              Remove <strong>{pendingRemoval.row.label}</strong>? The stored
              secret will be deleted from the system Keychain. This can’t be
              undone.
            </p>
          }
          confirmLabel="Remove"
          confirmDanger
          onCancel={() => setPendingRemoval(null)}
          onConfirm={() => void confirmRemoval()}
        />
      )}

      {adding && (
        <AddKeyModal
          accounts={accounts}
          onClose={() => setAdding(false)}
          onAdded={added}
        />
      )}

      {usageModalFor && (
        <OAuthUsageModal
          token={usageModalFor}
          onClose={() => {
            setUsageModalFor(null);
            // `key_oauth_usage` updates the probe status server-side
            // (last_probed_at, last_probe_status). Re-fetch so the
            // row's days-left chip + "Revoked" state reflect reality.
            void refresh();
          }}
        />
      )}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}

export const CLIPBOARD_CLEAR_MS = 30_000;

/** Overwrite the clipboard with an empty string 30s after a secret
 *  was copied, but only if the clipboard still holds that exact
 *  token — avoids stomping on whatever the user copied next. If
 *  `readText` is denied we can't verify the clipboard still holds
 *  our token, so we abort rather than blind-clear whatever replaced
 *  it. The toast already told the user to expect a 30s clear; a
 *  no-op beats clobbering their next copy.
 *
 *  Exported so the behavior is unit-testable with fake timers. */
export function scheduleClipboardClear(token: string): void {
  window.setTimeout(async () => {
    try {
      const current = await navigator.clipboard.readText();
      if (current !== token) return;
      await navigator.clipboard.writeText("");
    } catch {
      // readback denied — can't prove the clipboard still holds the
      // token, so don't risk clobbering whatever's there now.
    }
  }, CLIPBOARD_CLEAR_MS);
}

function describeProbe(status: string | null): string {
  if (!status) return "no probe yet";
  if (status === "ok") return "token is valid";
  if (status === "unauthorized") return "token rejected (expired or revoked)";
  if (status.startsWith("rate_limited:")) {
    const secs = status.split(":")[1];
    return `rate-limited (retry in ${secs}s)`;
  }
  if (status.startsWith("error:")) return status.slice("error:".length);
  return status;
}

/* ──────────────────────────────────────────────────────────── */
/*                         Tables                              */
/* ──────────────────────────────────────────────────────────── */

function ApiKeysTable({
  rows,
  loading,
  onCopy,
  onRemove,
}: {
  rows: ApiKeySummary[];
  loading: boolean;
  onCopy: (row: ApiKeySummary) => void;
  onRemove: (row: ApiKeySummary) => void;
}) {
  return (
    <section>
      <SectionLabel>
        API keys {rows.length > 0 ? `· ${rows.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        Console-issued <code>sk-ant-api03-…</code> keys. Usage reports are
        not available per-key via the public API.
      </p>

      {loading && rows.length === 0 ? (
        <EmptyHint>Loading…</EmptyHint>
      ) : rows.length === 0 ? (
        <EmptyHint>
          No API keys yet. Add one from your{" "}
          <ExternalLink href="https://console.anthropic.com/settings/keys">
            Anthropic console
          </ExternalLink>
          .
        </EmptyHint>
      ) : (
        <Table>
          <Thead
            cols={["Label", "Preview", "Created by", "Created", ""]}
          />
          <tbody>
            {rows.map((row) => (
              <Tr key={row.uuid}>
                <Td>
                  <strong style={{ fontWeight: 600 }}>{row.label}</strong>
                </Td>
                <Td>
                  <code style={{ fontSize: "var(--fs-xs)" }}>
                    {row.token_preview}
                  </code>
                </Td>
                <Td>
                  {row.account_email ? (
                    <Tag tone="neutral">{row.account_email}</Tag>
                  ) : (
                    <Tag
                      tone="warn"
                      title="The account this key was created under has been removed."
                    >
                      account removed
                    </Tag>
                  )}
                </Td>
                <Td>
                  <span
                    style={{
                      fontSize: "var(--fs-xs)",
                      color: "var(--fg-muted)",
                    }}
                  >
                    {fmtDate(row.created_at)}
                  </span>
                </Td>
                <Td align="right">
                  <RowActions>
                    <IconButton
                      glyph={NF.copy}
                      title="Copy full value to clipboard"
                      aria-label={`Copy ${row.label}`}
                      onClick={() => onCopy(row)}
                    />
                    <IconButton
                      glyph={NF.trash}
                      title="Remove"
                      aria-label={`Remove ${row.label}`}
                      onClick={() => onRemove(row)}
                    />
                  </RowActions>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      )}
    </section>
  );
}

function OauthTokensTable({
  rows,
  loading,
  onCopy,
  onRemove,
  onProbe,
  onOpenUsage,
}: {
  rows: OauthTokenSummary[];
  loading: boolean;
  onCopy: (row: OauthTokenSummary) => void;
  onRemove: (row: OauthTokenSummary) => void;
  onProbe: (row: OauthTokenSummary) => void;
  onOpenUsage: (row: OauthTokenSummary) => void;
}) {
  return (
    <section>
      <SectionLabel>
        OAuth tokens {rows.length > 0 ? `· ${rows.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        Long-lived <code>sk-ant-oat01-…</code> tokens generated by{" "}
        <code>claude setup-token</code>. Click the tag to view usage.
      </p>

      {loading && rows.length === 0 ? (
        <EmptyHint>Loading…</EmptyHint>
      ) : rows.length === 0 ? (
        <EmptyHint>
          No OAuth tokens yet. Run <code>claude setup-token</code> and paste
          the value into “Add key”.
        </EmptyHint>
      ) : (
        <Table>
          <Thead
            cols={[
              "Label",
              "Preview",
              "Created by",
              "Created",
              "Expires",
              "",
            ]}
          />
          <tbody>
            {rows.map((row) => (
              <Tr key={row.uuid}>
                <Td>
                  <strong style={{ fontWeight: 600 }}>{row.label}</strong>
                </Td>
                <Td>
                  <code style={{ fontSize: "var(--fs-xs)" }}>
                    {row.token_preview}
                  </code>
                </Td>
                <Td>
                  <button
                    type="button"
                    onClick={() => onOpenUsage(row)}
                    title="View usage"
                    style={{
                      background: "transparent",
                      border: "none",
                      padding: 0,
                      cursor: "pointer",
                    }}
                  >
                    <Tag tone="accent">
                      {row.account_email ?? row.account_uuid.slice(0, 8)}
                    </Tag>
                  </button>
                </Td>
                <Td>
                  <span
                    style={{
                      fontSize: "var(--fs-xs)",
                      color: "var(--fg-muted)",
                    }}
                  >
                    {fmtDate(row.created_at)}
                  </span>
                </Td>
                <Td>
                  <DaysLeftChip
                    daysRemaining={row.days_remaining}
                    probeStatus={row.last_probe_status}
                  />
                </Td>
                <Td align="right">
                  <RowActions>
                    <IconButton
                      glyph={NF.refresh}
                      title="Probe (verify validity)"
                      aria-label={`Probe ${row.label}`}
                      onClick={() => onProbe(row)}
                    />
                    <IconButton
                      glyph={NF.copy}
                      title="Copy full value to clipboard"
                      aria-label={`Copy ${row.label}`}
                      onClick={() => onCopy(row)}
                    />
                    <IconButton
                      glyph={NF.trash}
                      title="Remove"
                      aria-label={`Remove ${row.label}`}
                      onClick={() => onRemove(row)}
                    />
                  </RowActions>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      )}
    </section>
  );
}

function DaysLeftChip({
  daysRemaining,
  probeStatus,
}: {
  daysRemaining: number;
  probeStatus: string | null;
}) {
  if (probeStatus === "unauthorized") {
    return (
      <Tag tone="danger" glyph={NF.xCircle}>
        Revoked
      </Tag>
    );
  }
  if (daysRemaining <= 0) {
    return (
      <Tag tone="danger" glyph={NF.xCircle}>
        Expired
      </Tag>
    );
  }
  if (daysRemaining < 30) {
    return (
      <Tag tone="warn" glyph={NF.warn}>
        {daysRemaining}d left
      </Tag>
    );
  }
  return (
    <Tag tone="neutral" glyph={NF.clock}>
      {daysRemaining}d left
    </Tag>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         Primitives                          */
/* ──────────────────────────────────────────────────────────── */

function Table({ children }: { children: React.ReactNode }) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </table>
  );
}

function Thead({ cols }: { cols: string[] }) {
  return (
    <thead>
      <tr>
        {cols.map((c, i) => (
          <th
            key={i}
            className="mono-cap"
            style={{
              padding: "var(--sp-8) var(--sp-10)",
              textAlign: i === cols.length - 1 ? "right" : "left",
              fontSize: "var(--fs-xs)",
              fontWeight: 500,
              color: "var(--fg-faint)",
              borderBottom: "var(--bw-hair) solid var(--line)",
            }}
          >
            {c}
          </th>
        ))}
      </tr>
    </thead>
  );
}

function Tr({ children }: { children: React.ReactNode }) {
  return (
    <tr
      style={{
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      {children}
    </tr>
  );
}

function Td({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: "left" | "right";
}) {
  return (
    <td
      style={{
        padding: "var(--sp-10)",
        textAlign: align ?? "left",
        verticalAlign: "middle",
      }}
    >
      {children}
    </td>
  );
}

function RowActions({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        display: "inline-flex",
        gap: "var(--sp-4)",
        justifyContent: "flex-end",
      }}
    >
      {children}
    </span>
  );
}

function EmptyHint({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: "var(--sp-24) var(--sp-16)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--r-2)",
        textAlign: "center",
        fontSize: "var(--fs-sm)",
        color: "var(--fg-muted)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: "var(--sp-6)",
      }}
    >
      <Glyph g={NF.key} color="var(--fg-faint)" />
      {children}
    </div>
  );
}

function fmtDate(rfc: string): string {
  const d = new Date(rfc);
  if (Number.isNaN(d.getTime())) return rfc;
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
