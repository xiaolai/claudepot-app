import { useCallback, useEffect, useMemo, useState } from "react";
import type { AccountSummary } from "../types";
import { api } from "../api";
import { useUsage } from "../hooks/useUsage";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useCompactHeader } from "../hooks/useWindowWidth";
import { useAppState } from "../providers/AppStateProvider";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { NF } from "../icons";
import { SkeletonList } from "../components/primitives/Skeleton";
import { ScreenHeader } from "../shell/ScreenHeader";
import { AccountsGrid } from "./accounts/AccountsGrid";
import { AddAccountModal } from "./accounts/AddAccountModal";
import { HealthChips } from "./accounts/HealthChips";
import { CtxMenuForAccount } from "./accounts/useAccountContextMenu";
import { useAccountHandlers } from "./accounts/useAccountHandlers";
import type {
  CliTargetHandlers,
  DesktopTargetHandlers,
} from "./accounts/targetButtonStates";

/**
 * Accounts section. Renders the header, filter bar, and the card grid.
 * Refresh/toast state is lifted to `AppStateProvider` — the shell-level
 * `StatusIssuesBanner` and this section share the same `/profile` and
 * `verify_all_accounts` traffic off one useRefresh instance. Per-view
 * state (usage cache, busy keys, modals, palette) stays local.
 */
export function AccountsSection({
  onNavigate,
}: {
  onNavigate?: (section: string, subRoute?: string | null) => void;
}) {
  const {
    pushToast,
    status,
    accounts,
    ccIdentity,
    loadError,
    refresh,
    actions,
    busyKeys,
    requestCliSwap,
    requestRemoveAccount,
    requestDesktopOverwrite,
  } = useAppState();
  const { usage, refreshUsage, refreshUsageFor, lastFetchedAt } = useUsage();
  const compact = useCompactHeader();

  // Tick once a minute so the "updated Xm ago" label ages without a
  // full section re-render. Cheap — a single state bump per tick.
  const [, setNowTick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => setNowTick((n) => n + 1), 60_000);
    return () => window.clearInterval(id);
  }, []);
  const usageAgeLabel = useMemo(
    () => formatUsageAge(lastFetchedAt),
    [lastFetchedAt],
  );

  // Token counts per account — one fetch on mount. Keys section owns
  // the full lifecycle; this is a read-only decoration on the
  // Accounts cards. Quiet failure: if the backend doesn't answer
  // (e.g. first-run, no keychain), the chip just doesn't render.
  const [tokenCounts, setTokenCounts] = useState<Record<string, number>>({});
  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.keyApiList().catch(() => []),
      api.keyOauthList().catch(() => []),
    ]).then(([apiKeys, oauthTokens]) => {
      if (cancelled) return;
      const counts: Record<string, number> = {};
      for (const k of apiKeys) {
        if (k.account_uuid) {
          counts[k.account_uuid] = (counts[k.account_uuid] ?? 0) + 1;
        }
      }
      for (const t of oauthTokens) {
        if (t.account_uuid) {
          counts[t.account_uuid] = (counts[t.account_uuid] ?? 0) + 1;
        }
      }
      setTokenCounts(counts);
    });
    return () => {
      cancelled = true;
    };
  }, [accounts]);

  const handleOpenTokensFor = useCallback(
    (email: string) => {
      onNavigate?.("keys");
      // Dispatch on the next tick so KeysSection has mounted and its
      // cp-keys-filter listener is wired up before we fire.
      window.setTimeout(() => {
        window.dispatchEvent(
          new CustomEvent("cp-keys-filter", { detail: { query: email } }),
        );
      }, 0);
    },
    [onNavigate],
  );

  const [showAdd, setShowAdd] = useState(false);
  const [filter, setFilter] = useState("");
  const [ctxMenu, setCtxMenu] = useState<
    | { kind: "row"; x: number; y: number; account: AccountSummary }
    | null
  >(null);

  const { runVerifyAccount, runVerifyAll, handleDesktopSwitch } =
    useAccountHandlers({
      pushToast,
      refresh,
      useDesktop: actions.useDesktop,
    });

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.preventDefault();
      setCtxMenu({ kind: "row", x: e.clientX, y: e.clientY, account: a });
    },
    [],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  // Cmd+Shift+C — copy first matching email when a filter is active,
  // else the first account in the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && e.key === "c") {
        e.preventDefault();
        const target = shown[0];
        if (!target) return;
        void navigator.clipboard
          .writeText(target.email)
          .then(() => pushToast("info", `Copied ${target.email}`))
          .catch((err) => pushToast("error", `Copy failed: ${err}`));
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
  });

  // Add-account bridge — the macOS app menu and the tray menu both
  // dispatch this to open the AddAccountModal from outside the section.
  useEffect(() => {
    const onOpen = () => setShowAdd(true);
    window.addEventListener("cp-open-add", onOpen);
    return () => window.removeEventListener("cp-open-add", onOpen);
  }, []);

  // Shell-level drift banners deep-link into a specific account via
  // `cp-focus-account`. The CustomEvent payload is the target UUID;
  // we find the matching card by data attribute and bring it into view.
  useEffect(() => {
    const onFocus = (e: Event) => {
      const uuid = (e as CustomEvent<string>).detail;
      if (!uuid) return;
      // Clear any filter that would hide the target row so the scroll
      // target is actually mounted.
      setFilter("");
      requestAnimationFrame(() => {
        const el = document.querySelector<HTMLElement>(
          `[data-account-uuid="${uuid}"]`,
        );
        el?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
    };
    window.addEventListener("cp-focus-account", onFocus);
    return () => window.removeEventListener("cp-focus-account", onFocus);
  }, []);

  const trayRefreshAll = useCallback(() => {
    refresh();
    refreshUsage();
  }, [refresh, refreshUsage]);
  // Split the two channels: the shell-level App listener already
  // calls `refreshAccounts()` on `tray-cli-switched`, so this
  // section only needs to refetch the per-account usage chips. A
  // CLI swap doesn't change usage data, but the tray's Usage submenu
  // re-emits `rebuild-tray-menu` after a refresh and that path
  // expects the section's cache to be primed. `tray-refresh-requested`
  // is the broader "something material changed" signal; keep both
  // refreshes there. The "needs-override" branch is gone: the tray
  // now always forces the swap (the SplitBrainConfirm modal it used
  // to raise was invisible when the window was hidden, which is the
  // failure mode that drove this change). User-visible feedback for
  // the tray switch (toast + OS notification + Undo on success, OS
  // notification on failure) lives in the App shell.
  useTauriEvent("tray-cli-switched", refreshUsage);
  useTauriEvent("tray-refresh-requested", trayRefreshAll);

  const shown = useMemo(() => {
    if (!filter.trim()) return accounts;
    const q = filter.toLowerCase();
    return accounts.filter(
      (a) =>
        a.email.toLowerCase().includes(q) ||
        a.org_name?.toLowerCase().includes(q),
    );
  }, [accounts, filter]);

  // Handler bags routed into each AccountCard's TargetButtons. The
  // adopt path still branches on `desktop_profile_on_disk` the same
  // way CtxMenuForAccount does — when a snapshot already exists the
  // shell-level overwrite confirm owns the decision.
  const cliHandlers: CliTargetHandlers = useMemo(
    () => ({
      switchCli: (a) => requestCliSwap(a),
      verify: (a) => runVerifyAccount(a),
      login: (a) => actions.login(a),
    }),
    [requestCliSwap, runVerifyAccount, actions],
  );
  const desktopHandlers: DesktopTargetHandlers = useMemo(
    () => ({
      switchDesktop: (a) => handleDesktopSwitch(a),
      switchDesktopNoLaunch: (a) => void actions.useDesktop(a, true),
      launchDesktop: () => {
        api.desktopLaunch().catch((e) => {
          const msg = e instanceof Error ? e.message : String(e);
          pushToast("error", `Desktop launch failed: ${msg}`);
        });
      },
      adoptDesktop: (a) => {
        if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
        else void actions.adoptDesktop(a);
      },
    }),
    [handleDesktopSwitch, actions, requestDesktopOverwrite, pushToast],
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
            maxWidth: "var(--content-cap-md)",
            margin: "0 auto",
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
            Couldn't load accounts
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-sm)",
              margin: 0,
              textAlign: "center",
            }}
          >
            Claudepot couldn't read its account database. Retrying often
            resolves this; if it persists, check the data directory in
            Settings → Diagnostics.
          </p>
          <Button variant="solid" onClick={() => refresh()}>
            Retry
          </Button>
          <details style={{ width: "100%" }}>
            <summary
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
                cursor: "pointer",
                textTransform: "uppercase",
                letterSpacing: "var(--ls-wide)",
              }}
            >
              Error detail
            </summary>
            <pre
              style={{
                margin: "var(--sp-6) 0 0",
                padding: "var(--sp-8)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-muted)",
                background: "var(--bg-sunken)",
                borderRadius: "var(--r-1)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
              }}
            >
              {loadError}
            </pre>
          </details>
        </div>
      );
    }
    return (
      <SkeletonList
        rows={2}
        label="Loading accounts…"
        style={{ padding: "var(--sp-32)" }}
      />
    );
  }

  return (
    <>
      <ScreenHeader
        title="Accounts"
        subtitle={<HealthChips accounts={accounts} />}
        actions={
          <>
            {compact ? (
              <>
                <IconButton
                  glyph={NF.shield}
                  onClick={runVerifyAll}
                  title="Verify all — check every account against /profile"
                  aria-label="Verify all accounts"
                />
                <IconButton
                  glyph={NF.refresh}
                  onClick={() => {
                    refresh();
                    refreshUsage();
                  }}
                  title="Refresh usage (⌘R)"
                  aria-label="Refresh usage"
                />
              </>
            ) : (
              <>
                <Button
                  variant="ghost"
                  glyph={NF.shield}
                  glyphColor="var(--fg-muted)"
                  onClick={runVerifyAll}
                  title="Verify every account against /profile"
                >
                  Verify all
                </Button>
                {usageAgeLabel && (
                  <span
                    className="mono-cap"
                    style={{
                      fontSize: "var(--fs-2xs)",
                      color: "var(--fg-faint)",
                      letterSpacing: "var(--ls-wide)",
                    }}
                    title={
                      lastFetchedAt
                        ? new Date(lastFetchedAt).toLocaleString()
                        : undefined
                    }
                  >
                    {usageAgeLabel}
                  </span>
                )}
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
              </>
            )}
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

      <AccountsGrid
        accounts={accounts}
        shown={shown}
        usage={usage}
        status={status}
        busyKeys={busyKeys}
        filter={filter}
        onFilterChange={setFilter}
        onLogin={actions.login}
        onContextMenu={handleContextMenu}
        cliHandlers={cliHandlers}
        desktopHandlers={desktopHandlers}
        ccIdentity={ccIdentity}
        tokenCounts={tokenCounts}
        onOpenTokens={handleOpenTokensFor}
        onAdd={() => setShowAdd(true)}
        onAdoptCurrent={async () => {
          try {
            const outcome = await api.accountAddFromCurrent();
            pushToast("info", `Adopted ${outcome.email}.`);
            await refresh();
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            pushToast("error", `Couldn't adopt current session: ${msg}`);
          }
        }}
      />

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
        onAdoptDesktop={(a) => actions.adoptDesktop(a)}
      />

      {ctxMenu && (
        <CtxMenuForAccount
          menu={ctxMenu}
          status={status}
          busyKeys={busyKeys}
          onSwitchCli={requestCliSwap}
          onSwitchDesktop={handleDesktopSwitch}
          onSwitchDesktopNoLaunch={(a) => actions.useDesktop(a, true)}
          onVerify={runVerifyAccount}
          onRefreshUsageFor={(a) => refreshUsageFor(a.uuid)}
          onRefreshUsageAll={refreshUsage}
          onLogin={actions.login}
          onRemove={requestRemoveAccount}
          onAdoptDesktop={(a) => {
            // Adopt with no overwrite by default. If a snapshot
            // already exists for this account, go through the
            // shell-level confirm — the user must opt into
            // replacing the existing profile.
            if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
            else void actions.adoptDesktop(a);
          }}
          onClose={closeCtxMenu}
        />
      )}

    </>
  );
}

/**
 * Compact "updated 12m ago" label for the Accounts header. Returns
 * null when no fetch has happened yet, "just now" for < 30 s, else
 * minutes/hours. Seconds are suppressed on purpose — the label is a
 * freshness cue, not a stopwatch.
 */
function formatUsageAge(lastFetchedAt: number | null): string | null {
  if (!lastFetchedAt) return null;
  const deltaMs = Date.now() - lastFetchedAt;
  if (deltaMs < 30_000) return "updated just now";
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 1) return "updated just now";
  if (minutes < 60) return `updated ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `updated ${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `updated ${days}d ago`;
}
