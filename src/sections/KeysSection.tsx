import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { SkeletonRows } from "../components/primitives/Skeleton";
import { Table, Th, Tr, Td } from "../components/primitives/Table";
import { Tag } from "../components/primitives/Tag";
import { useAppState } from "../providers/AppStateProvider";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import type {
  AccountSummaryBasic,
  ApiKeySummary,
  OauthTokenSummary,
} from "../types";
import { AddKeyModal } from "./keys/AddKeyModal";
import { OAuthUsageModal } from "./keys/OAuthUsageModal";

type PendingRemoval =
  | { kind: "api"; row: ApiKeySummary }
  | { kind: "oauth"; row: OauthTokenSummary };

export function KeysSection() {
  const { pushToast } = useAppState();
  const [apiKeys, setApiKeys] = useState<ApiKeySummary[]>([]);
  const [oauthTokens, setOauthTokens] = useState<OauthTokenSummary[]>([]);
  const [accounts, setAccounts] = useState<AccountSummaryBasic[]>([]);
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
      // Keys only needs identity fields (uuid → email) to label each
      // row's owner. The full `accountList` issues one macOS Keychain
      // syscall per account for token-health computation and runs a
      // reconcile pass on top — that stall was what made this tab
      // feel semi-frozen on mount. Basic variant is pure sqlite.
      const [api_, oauth, accts] = await Promise.all([
        api.keyApiList(),
        api.keyOauthList(),
        api.accountListBasic(),
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

  // Cross-section deep-link: AccountCard's "N tokens" chip dispatches
  // `cp-keys-filter` so this section can land pre-filtered to an
  // account. Payload is the literal filter query (typically an email).
  useEffect(() => {
    const onFilter = (e: Event) => {
      const detail = (e as CustomEvent<{ query: string }>).detail;
      if (typeof detail?.query === "string") setFilter(detail.query);
    };
    window.addEventListener("cp-keys-filter", onFilter);
    return () => window.removeEventListener("cp-keys-filter", onFilter);
  }, []);

  // D-5/6/7: secret never enters JS. Rust writes the clipboard
  // directly and schedules its own 30s self-clear; we just toast the
  // receipt the bridge hands back (label + preview).
  const onCopy = useCallback(
    async (kind: "api" | "oauth", uuid: string) => {
      try {
        const r =
          kind === "api"
            ? await api.keyApiCopy(uuid)
            : await api.keyOauthCopy(uuid);
        pushToast(
          "info",
          `Copied ${r.label} (${r.preview}) — clipboard clears in 30s.`,
        );
      } catch (e) {
        pushToast("error", `Copy failed: ${e}`);
      }
    },
    [pushToast],
  );

  // Paste-ready POSIX shell invocation. The format string is built
  // server-side (`key_oauth_copy_shell`) so the raw token never
  // crosses the IPC bridge. CC reads `CLAUDE_CODE_OAUTH_TOKEN` first
  // (auth.ts:168, 1260) and never touches the keychain — letting the
  // user open a new terminal, paste, and switch identities without
  // disturbing the current login.
  const onCopyShell = useCallback(
    async (row: OauthTokenSummary) => {
      try {
        const r = await api.keyOauthCopyShell(row.uuid);
        pushToast(
          "info",
          `Copied shell command for ${r.label} (${r.preview}) — clipboard clears in 30s.`,
        );
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


  const onRename = useCallback(
    async (kind: "api" | "oauth", uuid: string, label: string) => {
      try {
        if (kind === "api") await api.keyApiRename(uuid, label);
        else await api.keyOauthRename(uuid, label);
        if (kind === "api") {
          setApiKeys((rows) =>
            rows.map((r) => (r.uuid === uuid ? { ...r, label } : r)),
          );
        } else {
          setOauthTokens((rows) =>
            rows.map((r) => (r.uuid === uuid ? { ...r, label } : r)),
          );
        }
      } catch (e) {
        pushToast("error", `Rename failed: ${e}`);
        throw e;
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
          onCopy={(row) => void onCopy("api", row.uuid)}
          onProbe={(row) =>
            void api
              .keyApiProbe(row.uuid)
              .then(() => pushToast("info", `${row.label}: valid`))
              .catch((e) => pushToast("error", `${row.label}: ${e}`))
          }
          onRemove={(row) => setPendingRemoval({ kind: "api", row })}
          onRename={(row, label) => onRename("api", row.uuid, label)}
          onAddRequested={() => setAdding(true)}
        />

        <OauthTokensTable
          rows={shownOauth}
          loading={loading}
          onCopy={(row) => void onCopy("oauth", row.uuid)}
          onCopyShell={(row) => void onCopyShell(row)}
          onRemove={(row) => setPendingRemoval({ kind: "oauth", row })}
          onOpenUsage={setUsageModalFor}
          onRename={(row, label) => onRename("oauth", row.uuid, label)}
          onAddRequested={() => setAdding(true)}
        />
      </main>

      {pendingRemoval && (
        <ConfirmDialog
          title="Remove key?"
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
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
    </>
  );
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
  onRename,
  onAddRequested,
}: {
  rows: ApiKeySummary[];
  loading: boolean;
  onCopy: (row: ApiKeySummary) => void;
  onProbe: (row: ApiKeySummary) => void;
  onRemove: (row: ApiKeySummary) => void;
  onRename: (row: ApiKeySummary, label: string) => Promise<void>;
  onAddRequested: () => void;
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
        <SkeletonRows rows={3} />
      ) : rows.length === 0 ? (
        <EmptyHint>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)", alignItems: "flex-start" }}>
            <span>
              No API keys yet. Create one in your{" "}
              <ExternalLink href="https://console.anthropic.com/settings/keys">
                Anthropic console
              </ExternalLink>
              , then paste it here.
            </span>
            <Button
              variant="ghost"
              glyph={NF.plus}
              onClick={onAddRequested}
            >
              Add API key
            </Button>
          </div>
        </EmptyHint>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Label</Th>
              <Th>Created by</Th>
              <Th>Created</Th>
              <Th align="right" aria-label="Actions" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <Tr key={row.uuid}>
                <Td>
                  <EditableLabel
                    value={row.label}
                    onSubmit={(label) => onRename(row, label)}
                  />
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
  onRename,
  onAddRequested,
}: {
  rows: OauthTokenSummary[];
  loading: boolean;
  onCopy: (row: OauthTokenSummary) => void;
  onCopyShell: (row: OauthTokenSummary) => void;
  onRemove: (row: OauthTokenSummary) => void;
  onOpenUsage: (row: OauthTokenSummary) => void;
  onRename: (row: OauthTokenSummary, label: string) => Promise<void>;
  onAddRequested: () => void;
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
        <SkeletonRows rows={3} />
      ) : rows.length === 0 ? (
        <EmptyHint>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)", alignItems: "flex-start" }}>
            <span>
              No OAuth tokens yet. Run <code>claude setup-token</code> and
              paste the value into “Add key”.
            </span>
            <Button
              variant="ghost"
              glyph={NF.plus}
              onClick={onAddRequested}
            >
              Add OAuth token
            </Button>
          </div>
        </EmptyHint>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Label</Th>
              <Th>Created by</Th>
              <Th>Created</Th>
              <Th>Expires</Th>
              <Th
                title={
                  "Copy a paste-ready terminal command " +
                  "(CLAUDE_CODE_OAUTH_TOKEN='…' claude). " +
                  "Launches Claude Code with this token in a new " +
                  "terminal without disturbing your current login."
                }
              >
                Shell{" "}
                <Glyph g={NF.info} color="var(--fg-faint)" size="var(--fs-xs)" />
              </Th>
              <Th align="right" aria-label="Actions" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <Tr key={row.uuid}>
                <Td>
                  <EditableLabel
                    value={row.label}
                    onSubmit={(label) => onRename(row, label)}
                  />
                </Td>
                <Td>
                  <button
                    type="button"
                    onClick={() => onOpenUsage(row)}
                    title={
                      row.account_email
                        ? "View usage"
                        : "View cached usage (linked account has been removed)"
                    }
                    style={{
                      background: "transparent",
                      border: "none",
                      padding: 0,
                      cursor: "pointer",
                    }}
                  >
                    <Tag
                      tone={row.account_email ? "accent" : "warn"}
                      style={{ textTransform: "none", letterSpacing: "normal" }}
                    >
                      {row.account_email ?? "account removed"}
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
/*                         EditableLabel                        */
/* ──────────────────────────────────────────────────────────── */

/** Always-present `<input>` that masquerades as text until focused.
 *  Font, color, padding, and border-bottom are reserved identically
 *  in both idle and edit states — focus only swaps the border-bottom
 *  color from transparent to accent, so the row does not shift by a
 *  single pixel. Blur commits; Enter blurs; Esc reverts then blurs.
 *  Empty/whitespace is treated as a no-op (backend rejects it and
 *  a blank label is never useful). */
function EditableLabel({
  value,
  onSubmit,
}: {
  value: string;
  onSubmit: (label: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(value);
  const [focused, setFocused] = useState(false);
  const [busy, setBusy] = useState(false);

  // Prop changes (parent-driven rename, refresh) win over local draft
  // whenever the field is not being actively edited.
  useEffect(() => {
    if (!focused) setDraft(value);
  }, [value, focused]);

  const commit = async () => {
    const next = draft.trim();
    if (!next || next === value) {
      setDraft(value);
      return;
    }
    setBusy(true);
    try {
      await onSubmit(next);
    } catch {
      setDraft(value);
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setDraft(value);
      // blur on next frame so the reverted draft is what `commit`
      // sees (and therefore early-returns).
      const el = e.currentTarget;
      requestAnimationFrame(() => el.blur());
    }
  };

  return (
    <input
      value={draft}
      disabled={busy}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => {
        setFocused(true);
        e.currentTarget.select();
      }}
      onBlur={() => {
        setFocused(false);
        void commit();
      }}
      onKeyDown={onKeyDown}
      aria-label="Key label"
      style={{
        width: "100%",
        font: "inherit",
        fontWeight: 600,
        color: "inherit",
        background: "transparent",
        border: 0,
        borderBottom: `var(--bw-hair) solid ${
          focused ? "var(--accent-border)" : "transparent"
        }`,
        padding: 0,
        margin: 0,
        outline: "none",
        cursor: "text",
      }}
    />
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                       Local helpers                         */
/* ──────────────────────────────────────────────────────────── */

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
