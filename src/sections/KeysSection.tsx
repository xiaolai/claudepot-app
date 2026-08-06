import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../api";
import { renderError } from "../lib/i18n-error";
import { formatDate } from "../lib/intl";
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
import { EnvVaultSection } from "./keys/EnvVaultSection";
import { consumePendingKeysFilter } from "./keys/pendingFilter";

type PendingRemoval =
  | { kind: "api"; row: ApiKeySummary }
  | { kind: "oauth"; row: OauthTokenSummary };

export function KeysSection() {
  const { t } = useTranslation("keys");
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
      pushToast("error", renderError(e, t("toasts.loadFailed")));
    } finally {
      setLoading(false);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Cross-section deep-link: AccountCard's "N tokens" chip stages a
  // query in the pending store (consumed here on mount — covers the
  // lazy-load path where this section wasn't mounted when the chip
  // was clicked) AND dispatches `cp-keys-filter` (covers an
  // already-mounted section). The event handler also drains the
  // pending slot so a query never goes stale and re-applies on a
  // later mount. Payload is the literal filter query (typically an
  // email).
  useEffect(() => {
    const pending = consumePendingKeysFilter();
    if (pending != null) setFilter(pending);
    const onFilter = (e: Event) => {
      consumePendingKeysFilter();
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
          t("toasts.copied", { label: r.label, preview: r.preview }),
        );
      } catch (e) {
        pushToast("error", renderError(e, t("toasts.copyFailed")));
      }
    },
    [pushToast, t],
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
          t("toasts.copiedShell", { label: r.label, preview: r.preview }),
        );
      } catch (e) {
        pushToast("error", renderError(e, t("toasts.copyFailed")));
      }
    },
    [pushToast, t],
  );

  const confirmRemoval = useCallback(async () => {
    if (!pendingRemoval) return;
    const { kind, row } = pendingRemoval;
    try {
      if (kind === "api") await api.keyApiRemove(row.uuid);
      else await api.keyOauthRemove(row.uuid);
      pushToast("info", t("toasts.removed", { label: row.label }));
      await refresh();
    } catch (e) {
      pushToast("error", renderError(e, t("toasts.removeFailed")));
    } finally {
      setPendingRemoval(null);
    }
  }, [pendingRemoval, pushToast, refresh, t]);


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
        pushToast("error", renderError(e, t("toasts.renameFailed")));
        throw e;
      }
    },
    [pushToast, t],
  );

  const added = useCallback(
    (kind: "api" | "oauth") => {
      pushToast(
        "info",
        kind === "api" ? t("toasts.apiKeyAdded") : t("toasts.oauthTokenAdded"),
      );
      setAdding(false);
      void refresh();
    },
    [pushToast, refresh, t],
  );

  return (
    <>
      <ScreenHeader
        title={t("section.title")}
        subtitle={t("section.subtitle")}
        actions={
          <Button
            variant="solid"
            glyph={NF.plus}
            onClick={() => setAdding(true)}
          >
            {t("section.addKey")}
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
            placeholder={t("section.filterPlaceholder")}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: "var(--filter-input-width)" }}
            aria-label={t("section.filterPlaceholder")}
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
              .then(() =>
                pushToast("info", t("toasts.probeValid", { label: row.label })),
              )
              .catch((e) => pushToast("error", renderError(e, row.label)))
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

        <EnvVaultSection />
      </main>

      {pendingRemoval && (
        <ConfirmDialog
          title={t("section.removeTitle")}
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
              <Trans
                ns="keys"
                i18nKey="section.removeBody"
                values={{ label: pendingRemoval.row.label }}
                components={{ strong: <strong /> }}
              />
            </p>
          }
          confirmLabel={t("section.removeConfirm")}
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
  const { t } = useTranslation("keys");
  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
        {t("list.apiTitle")} {rows.length > 0 ? `· ${rows.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        <Trans ns="keys" i18nKey="list.apiDesc" components={{ code: <code /> }} />
      </p>

      {loading && rows.length === 0 ? (
        <SkeletonRows rows={3} />
      ) : rows.length === 0 ? (
        <EmptyHint>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)", alignItems: "flex-start" }}>
            <span>
              <Trans
                ns="keys"
                i18nKey="list.apiEmpty"
                components={{
                  consoleLink: (
                    <ExternalLink href="https://console.anthropic.com/settings/keys">
                      Anthropic console
                    </ExternalLink>
                  ),
                }}
              />
            </span>
            <Button
              variant="ghost"
              glyph={NF.plus}
              onClick={onAddRequested}
            >
              {t("list.addApiKey")}
            </Button>
          </div>
        </EmptyHint>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>{t("list.colLabel")}</Th>
              <Th>{t("list.colCreatedBy")}</Th>
              <Th>{t("list.colCreated")}</Th>
              <Th align="right" aria-label={t("list.actionsAria")} />
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
                      title={t("list.accountRemovedTitle")}
                    >
                      {t("list.accountRemoved")}
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
                      glyph={NF.shield}
                      title={t("list.probeTitle")}
                      aria-label={t("list.probeAria", { label: row.label })}
                      onClick={() => onProbe(row)}
                    />
                    <IconButton
                      glyph={NF.copy}
                      title={t("list.copyTitle")}
                      aria-label={t("list.copyAria", { label: row.label })}
                      onClick={() => onCopy(row)}
                    />
                    <IconButton
                      glyph={NF.trash}
                      title={t("list.removeTitle")}
                      aria-label={t("list.removeAria", { label: row.label })}
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
  const { t } = useTranslation("keys");
  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
        {t("list.oauthTitle")} {rows.length > 0 ? `· ${rows.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        <Trans
          ns="keys"
          i18nKey="list.oauthDesc"
          components={{ code: <code /> }}
        />
      </p>

      {loading && rows.length === 0 ? (
        <SkeletonRows rows={3} />
      ) : rows.length === 0 ? (
        <EmptyHint>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)", alignItems: "flex-start" }}>
            <span>
              <Trans
                ns="keys"
                i18nKey="list.oauthEmpty"
                components={{ code: <code /> }}
              />
            </span>
            <Button
              variant="ghost"
              glyph={NF.plus}
              onClick={onAddRequested}
            >
              {t("list.addOauthToken")}
            </Button>
          </div>
        </EmptyHint>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>{t("list.colLabel")}</Th>
              <Th>{t("list.colCreatedBy")}</Th>
              <Th>{t("list.colCreated")}</Th>
              <Th>{t("list.colExpires")}</Th>
              <Th title={t("list.shellColTitle")}>
                {t("list.colShell")}{" "}
                <Glyph g={NF.info} color="var(--fg-faint)" size="var(--fs-xs)" />
              </Th>
              <Th align="right" aria-label={t("list.actionsAria")} />
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
                        ? t("list.viewUsage")
                        : t("list.viewUsageCached")
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
                      {row.account_email ?? t("list.accountRemoved")}
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
                    title={t("list.copyShellTitle")}
                    aria-label={t("list.copyShellAria", { label: row.label })}
                  />
                </Td>
                <Td align="right">
                  <RowActions>
                    <IconButton
                      glyph={NF.copy}
                      title={t("list.copyTitle")}
                      aria-label={t("list.copyAria", { label: row.label })}
                      onClick={() => onCopy(row)}
                    />
                    <IconButton
                      glyph={NF.trash}
                      title={t("list.removeTitle")}
                      aria-label={t("list.removeAria", { label: row.label })}
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
  const { t } = useTranslation("keys");
  if (daysRemaining <= 0) {
    return (
      <Tag tone="danger" glyph={NF.xCircle}>
        {t("list.expired")}
      </Tag>
    );
  }
  if (daysRemaining < 30) {
    return (
      <Tag tone="warn" glyph={NF.warn}>
        {t("list.daysShort", { days: daysRemaining })}
      </Tag>
    );
  }
  return (
    <Tag tone="neutral" glyph={NF.clock}>
      {t("list.daysShort", { days: daysRemaining })}
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
  const { t } = useTranslation("keys");
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
      aria-label={t("list.keyLabelAria")}
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
  return formatDate(d, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
