import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { AccountSummary } from "../types";
import { useToasts } from "../hooks/useToasts";
import { useBusy } from "../hooks/useBusy";
import { useRefresh } from "../hooks/useRefresh";
import { useUsage } from "../hooks/useUsage";
import { useActions } from "../hooks/useActions";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { CommandPalette } from "../components/CommandPalette";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ToastContainer } from "../components/ToastContainer";
import { Button } from "../components/primitives/Button";
import { Glyph } from "../components/primitives/Glyph";
import { Input } from "../components/primitives/Input";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import { AccountCard } from "./accounts/AccountCard";
import { AddAccountModal } from "./accounts/AddAccountModal";
import { isAnomaly } from "./accounts/AnomalyBanner";

/**
 * Accounts section. Renders the header, filter bar, and the card grid.
 * State (accounts, usage, verify, toasts, busy-keys, modals, palette,
 * context menus) still lives here since the hooks were authored for
 * this scope — a future cleanup can hoist it into context providers,
 * but the shell's `useAccounts` already re-fetches on window focus so
 * the App-level copy stays close to this section's copy.
 */
export function AccountsSection({
  onNavigate,
}: {
  onNavigate?: (section: string, subRoute?: string | null) => void;
}) {
  const { toasts, pushToast, dismissToast } = useToasts();
  const busy = useBusy();
  const {
    status,
    accounts,
    loadError,
    ccIdentity: _ccIdentity,
    refresh,
  } = useRefresh(pushToast);
  const { usage, refreshUsage, refreshUsageFor } = useUsage();
  const actions = useActions({ pushToast, refresh, ...busy });

  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(
    null,
  );
  const [confirmClear, setConfirmClear] = useState(false);
  const [showPalette, setShowPalette] = useState(false);
  const [filter, setFilter] = useState("");
  const [ctxMenu, setCtxMenu] = useState<
    | { kind: "row"; x: number; y: number; account: AccountSummary }
    | null
  >(null);

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.preventDefault();
      setCtxMenu({ kind: "row", x: e.clientX, y: e.clientY, account: a });
    },
    [],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  const runVerifyAccount = useCallback(
    async (a: AccountSummary) => {
      try {
        await api.verifyAccount(a.uuid);
        pushToast("info", `Verified ${a.email}`);
        refresh();
      } catch (e) {
        pushToast("error", `Verify failed: ${e}`);
      }
    },
    [pushToast, refresh],
  );

  const handleDesktopSwitch = useCallback(
    (a: AccountSummary) => {
      pushToast("info", `Switching Desktop to ${a.email}…`, () => {}, {
        undoMs: 3000,
        dedupeKey: "desktop-switch",
        onCommit: () => actions.useDesktop(a),
      });
    },
    [actions, pushToast],
  );

  // Cmd+Shift+C — copy first matching email when a filter is active,
  // else the first account in the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && e.key === "c") {
        e.preventDefault();
        const target = shown[0];
        if (target) {
          navigator.clipboard.writeText(target.email);
          pushToast("info", `Copied ${target.email}`);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `shown` is computed below — tracked by accounts/filter deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accounts, filter, pushToast]);

  useGlobalShortcuts({
    onRefresh: () => {
      refresh();
      refreshUsage();
    },
    onAdd: () => setShowAdd(true),
    onPalette: () => setShowPalette(true),
  });

  // Command palette bridge — WindowChrome dispatches this event when
  // the ⌘K hint is clicked. App.tsx can't open the palette directly
  // because the palette component currently lives inside this section.
  useEffect(() => {
    const onOpen = () => setShowPalette(true);
    window.addEventListener("cp-open-palette", onOpen);
    return () => window.removeEventListener("cp-open-palette", onOpen);
  }, []);

  const trayRefresh = useCallback(() => {
    refresh();
    refreshUsage();
  }, [refresh, refreshUsage]);
  useTauriEvent("tray-cli-switched", trayRefresh);
  useTauriEvent("tray-refresh-requested", trayRefresh);
  useTauriEvent<string>("tray-cli-switch-failed", (e) => {
    const detail = typeof e?.payload === "string" ? e.payload : "unknown";
    pushToast("error", `Tray switch failed: ${detail}`);
  });

  const shown = useMemo(() => {
    if (!filter.trim()) return accounts;
    const q = filter.toLowerCase();
    return accounts.filter(
      (a) =>
        a.email.toLowerCase().includes(q) ||
        a.org_name?.toLowerCase().includes(q),
    );
  }, [accounts, filter]);

  const anomalyCount = useMemo(
    () => accounts.filter(isAnomaly).length,
    [accounts],
  );

  if (!status) {
    if (loadError) {
      return (
        <div
          style={{
            padding: "var(--sp-48)",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--sp-12)",
          }}
        >
          <h2
            style={{
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg)",
              margin: 0,
            }}
          >
            Couldn't load Claudepot
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-xs)",
              margin: 0,
            }}
          >
            {loadError}
          </p>
          <Button variant="solid" onClick={() => refresh()}>
            Retry
          </Button>
        </div>
      );
    }
    return (
      <div
        style={{
          padding: "var(--sp-48)",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Loading accounts…
      </div>
    );
  }

  const subtitle = (() => {
    const n = accounts.length;
    if (n === 0) return "No accounts registered yet.";
    const accountsLabel = `${n} account${n === 1 ? "" : "s"}`;
    return anomalyCount > 0
      ? `${accountsLabel} · ${anomalyCount} need${anomalyCount === 1 ? "s" : ""} attention`
      : `${accountsLabel} · all healthy`;
  })();

  return (
    <>
      <ScreenHeader
        title="Accounts"
        subtitle={subtitle}
        actions={
          <>
            <Button
              variant="ghost"
              glyph={NF.refresh}
              glyphColor="var(--fg-muted)"
              onClick={() => {
                refresh();
                refreshUsage();
              }}
              title="Refresh (⌘R)"
            >
              Refresh usage
            </Button>
            <Button
              variant="solid"
              glyph={NF.plus}
              onClick={() => setShowAdd(true)}
              title="Add account (⌘N)"
            >
              Add account
            </Button>
          </>
        }
      />

      {/* Filter input only earns its row when there are enough
          accounts to usefully narrow. With 1–3 accounts the input
          is pure chrome. Once a 4th lands, the filter appears. */}
      {accounts.length > 3 && (
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
            placeholder="Filter accounts"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: "var(--filter-input-width)" }}
            aria-label="Filter accounts"
          />
          {filter.trim() !== "" && (
            <span
              className="mono-cap"
              style={{
                color: "var(--fg-faint)",
                marginLeft: "var(--sp-4)",
              }}
            >
              {`${shown.length} / ${accounts.length}`}
            </span>
          )}
          <div style={{ flex: 1 }} />
        </div>
      )}

      <div
        style={{
          padding: "var(--sp-20) var(--sp-32) var(--sp-40)",
          display: "grid",
          gridTemplateColumns:
            "repeat(auto-fill, minmax(var(--content-cap-sm), 1fr))",
          gap: "var(--sp-16)",
        }}
      >
        {shown.map((a) => (
          <AccountCard
            key={a.uuid}
            account={a}
            usageEntry={usage[a.uuid] ?? null}
            loginBusy={busy.busyKeys.has(`re-${a.uuid}`)}
            onRemove={(x) => setConfirmRemove(x)}
            onLogin={(x) => actions.login(x)}
            onContextMenu={handleContextMenu}
          />
        ))}
        {shown.length === 0 && accounts.length > 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            No accounts match "{filter}".
          </div>
        )}
        {accounts.length === 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-10)",
              alignItems: "center",
            }}
          >
            <Glyph g={NF.users} size="var(--sp-32)" color="var(--fg-ghost)" />
            <p style={{ margin: 0 }}>No accounts yet.</p>
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              {"Click "}
              <b>Add account</b>
              {" to import Claude Code's current session."}
            </p>
          </div>
        )}
      </div>

      <AddAccountModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        accounts={accounts}
        onAdded={async () => {
          setShowAdd(false);
          await refresh();
          pushToast("info", "Account added.");
        }}
        onError={(msg) => pushToast("error", msg)}
      />

      {confirmRemove && (
        <ConfirmDialog
          title="Remove account?"
          confirmLabel="Remove"
          confirmDanger
          body={
            <>
              <p>
                Remove <strong>{confirmRemove.email}</strong>?
              </p>
              <p className="muted small">
                Deletes credentials and Desktop profile. Active
                CLI/Desktop pointers will be cleared.
              </p>
            </>
          }
          onCancel={() => setConfirmRemove(null)}
          onConfirm={async () => {
            const t = confirmRemove;
            setConfirmRemove(null);
            await actions.performRemove(t);
          }}
        />
      )}

      {confirmClear && (
        <ConfirmDialog
          title="Sign out of Claude Code?"
          confirmLabel="Clear"
          confirmDanger
          body={
            <p>
              Clears CC's credentials file. You'll need to sign in
              again.
            </p>
          }
          onCancel={() => setConfirmClear(false)}
          onConfirm={async () => {
            setConfirmClear(false);
            await actions.performClearCli();
          }}
        />
      )}

      {showPalette && status && (
        <CommandPalette
          accounts={accounts}
          status={status}
          onClose={() => setShowPalette(false)}
          onSwitchCli={(a) => actions.useCli(a)}
          onSwitchDesktop={(a) => handleDesktopSwitch(a)}
          onAdd={() => setShowAdd(true)}
          onRefresh={() => {
            refresh();
            refreshUsage();
          }}
          onRemove={(a) => setConfirmRemove(a)}
          onNavigate={onNavigate}
        />
      )}

      {ctxMenu &&
        (() => {
          const a = ctxMenu.account;
          const items: ContextMenuItem[] = [
            {
              label: "Copy email",
              onClick: () => navigator.clipboard.writeText(a.email),
            },
            {
              label: "Copy UUID",
              onClick: () => navigator.clipboard.writeText(a.uuid),
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: a.is_cli_active ? "Active CLI" : "Set as CLI",
              disabled: a.is_cli_active || !a.credentials_healthy,
              onClick: () => actions.useCli(a),
            },
            {
              label: a.is_desktop_active
                ? "Active Desktop"
                : "Set as Desktop",
              disabled:
                a.is_desktop_active ||
                !a.has_desktop_profile ||
                !status.desktop_installed,
              onClick: () => handleDesktopSwitch(a),
            },
            {
              label: "Set as Desktop (don't relaunch)",
              disabled:
                a.is_desktop_active ||
                !a.has_desktop_profile ||
                !status.desktop_installed,
              onClick: () => actions.useDesktop(a, true),
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Verify now",
              disabled: !a.credentials_healthy,
              onClick: () => runVerifyAccount(a),
            },
            {
              label: "Refresh usage",
              onClick: () => {
                if (a.credentials_healthy) refreshUsageFor(a.uuid);
                else refreshUsage();
              },
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Log in again…",
              disabled: busy.busyKeys.has(`re-${a.uuid}`),
              onClick: () => actions.login(a),
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Remove",
              danger: true,
              onClick: () => setConfirmRemove(a),
            },
          ];
          return (
            <ContextMenu
              x={ctxMenu.x}
              y={ctxMenu.y}
              items={items}
              onClose={closeCtxMenu}
            />
          );
        })()}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}
