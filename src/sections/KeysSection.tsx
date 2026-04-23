import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
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
  const [filter, setFilter] = useState("");

  const accountEmailByUuid = useMemo(() => {
    const m = new Map<string, string>();
    for (const a of accounts) m.set(a.uuid, a.email);
    return m;
  }, [accounts]);

  const matches = useCallback(
    (row: { label: string; token_preview: string; account_uuid?: string }) => {
      const q = filter.trim().toLowerCase();
      if (!q) return true;
      if (row.label.toLowerCase().includes(q)) return true;
      if (row.token_preview.toLowerCase().includes(q)) return true;
      const email = row.account_uuid
        ? accountEmailByUuid.get(row.account_uuid)
        : undefined;
      return !!email && email.toLowerCase().includes(q);
    },
    [filter, accountEmailByUuid],
  );

  const shownApi = useMemo(
    () => apiKeys.filter(matches),
    [apiKeys, matches],
  );
  const shownOauth = useMemo(
    () => oauthTokens.filter(matches),
    [oauthTokens, matches],
  );
  const totalRows = apiKeys.length + oauthTokens.length;
  const shownRows = shownApi.length + shownOauth.length;

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

  // Copy a paste-ready POSIX shell invocation:
  //   CLAUDE_CODE_OAUTH_TOKEN='<token>' claude
  // CC reads the env var first (auth.ts:168, 1260) and never touches
  // the keychain, so the user can open a new terminal, paste, and run
  // as a different identity without disturbing the current login.
  const onCopyShell = useCallback(
    async (row: OauthTokenSummary) => {
      try {
        const token = await api.keyOauthCopy(row.uuid);
        const cmd = `CLAUDE_CODE_OAUTH_TOKEN='${token}' claude`;
        await navigator.clipboard.writeText(cmd);
        pushToast(
          "info",
          `Copied shell command for ${row.label} (${row.token_preview}) — clipboard clears in 30s.`,
        );
        scheduleClipboardClear(cmd);
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

      {totalRows > 4 && (
        <div
          style={{
            padding: "var(--sp-14) var(--sp-32)",
            borderBottom: "var(--bw-hair) solid var(--line)",
            display: "flex",
            gap: "var(--sp-12)",
            alignItems: "center",
            background: "var(--bg)",
          }}
        >
          <Input
            glyph={NF.search}
            placeholder="Filter keys and tokens"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: "var(--filter-input-width)" }}
            aria-label="Filter keys and tokens"
          />
          {filter.trim() !== "" && (
            <span
              className="mono-cap"
              style={{ color: "var(--fg-faint)", marginLeft: "var(--sp-4)" }}
            >
              {`${shownRows} / ${totalRows}`}
            </span>
          )}
        </div>
      )}

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
          rows={shownApi}
          loading={loading}
          onCopy={(row) =>
            void onCopy("api", row.uuid, row.label, row.token_preview)
          }
          onProbe={(row) =>
            void api
              .keyApiProbe(row.uuid)
              .then(() => pushToast("info", `${row.label}: valid`))
              .catch((e) => pushToast("error", `${row.label}: ${e}`))
          }
          onRemove={(row) => setPendingRemoval({ kind: "api", row })}
        />

        <OauthTokensTable
          rows={shownOauth}
          loading={loading}
          onCopy={(row) =>
            void onCopy("oauth", row.uuid, row.label, row.token_preview)
          }
          onCopyShell={(row) => void onCopyShell(row)}
          onRemove={(row) => setPendingRemoval({ kind: "oauth", row })}
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

/* ──────────────────────────────────────────────────────────── */
/*                         Tables                              */
/* ──────────────────────────────────────────────────────────── */

function ApiKeysTable({
  rows,
  loading,
  onCopy,
  onProbe,
  onRemove,
}: {
  rows: ApiKeySummary[];
  loading: boolean;
  onCopy: (row: ApiKeySummary) => void;
  onProbe: (row: ApiKeySummary) => void;
  onRemove: (row: ApiKeySummary) => void;
}) {
  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
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
            cols={["Label", "Created by", "Created", ""]}
          />
          <tbody>
            {rows.map((row) => (
              <Tr key={row.uuid}>
                <Td>
                  <strong style={{ fontWeight: 600 }}>{row.label}</strong>
                </Td>
                <Td>
                  {row.account_email ? (
                    <Tag
                      tone="neutral"
                      style={{ textTransform: "none", letterSpacing: "normal" }}
                    >
                      {row.account_email}
                    </Tag>
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

function OauthTokensTable({
  rows,
  loading,
  onCopy,
  onCopyShell,
  onRemove,
  onOpenUsage,
}: {
  rows: OauthTokenSummary[];
  loading: boolean;
  onCopy: (row: OauthTokenSummary) => void;
  onCopyShell: (row: OauthTokenSummary) => void;
  onRemove: (row: OauthTokenSummary) => void;
  onOpenUsage: (row: OauthTokenSummary) => void;
}) {
  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
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
        <code>claude setup-token</code>.
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
              "Created by",
              "Created",
              "Expires",
              {
                label: "Shell",
                hint:
                  "Copy a paste-ready terminal command " +
                  "(CLAUDE_CODE_OAUTH_TOKEN='…' claude). " +
                  "Launches Claude Code with this token in a new " +
                  "terminal without disturbing your current login.",
              },
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
                    <Tag
                      tone="accent"
                      style={{ textTransform: "none", letterSpacing: "normal" }}
                    >
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
                  <DaysLeftChip daysRemaining={row.days_remaining} />
                </Td>
                <Td>
                  <IconButton
                    glyph={NF.terminal}
                    onClick={() => onCopyShell(row)}
                    title="Copy: CLAUDE_CODE_OAUTH_TOKEN='…' claude"
                    aria-label={`Copy shell command for ${row.label}`}
                  />
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

function DaysLeftChip({ daysRemaining }: { daysRemaining: number }) {
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
        {daysRemaining}d
      </Tag>
    );
  }
  return (
    <Tag tone="neutral" glyph={NF.clock}>
      {daysRemaining}d
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

type ColSpec = string | { label: string; hint: string };

function Thead({ cols }: { cols: ColSpec[] }) {
  return (
    <thead>
      <tr>
        {cols.map((c, i) => {
          const label = typeof c === "string" ? c : c.label;
          const hint = typeof c === "string" ? undefined : c.hint;
          return (
            <th
              key={i}
              className="mono-cap"
              title={hint}
              style={{
                padding: "var(--sp-8) var(--sp-10)",
                textAlign: i === cols.length - 1 ? "right" : "left",
                fontSize: "var(--fs-xs)",
                fontWeight: 500,
                color: "var(--fg-faint)",
                borderBottom: "var(--bw-hair) solid var(--line)",
                cursor: hint ? "help" : undefined,
              }}
            >
              {label}
              {hint && (
                <>
                  {" "}
                  <Glyph
                    g={NF.info}
                    color="var(--fg-faint)"
                    size="var(--fs-xs)"
                  />
                </>
              )}
            </th>
          );
        })}
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
