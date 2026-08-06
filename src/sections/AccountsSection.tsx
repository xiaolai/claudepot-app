import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AccountSummary } from "../types";
import { api } from "../api";
import { i18n } from "../lib/i18n";
import { renderError } from "../lib/i18n-error";
import { formatDateTime } from "../lib/intl";
import { useUsage } from "../hooks/useUsage";
import { useTauriEvent } from "../hooks/useTauriEvent";
import {
  isShortcutContextBlocked,
  useGlobalShortcuts,
} from "../hooks/useGlobalShortcuts";
import { useCompactHeader } from "../hooks/useWindowWidth";
import { useAppState } from "../providers/AppStateProvider";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { NF } from "../icons";
import { SkeletonList } from "../components/primitives/Skeleton";
import { ScreenHeader } from "../shell/ScreenHeader";
import { setPendingKeysFilter } from "./keys/pendingFilter";
import { AccountsGrid } from "./accounts/AccountsGrid";
import { AddAccountModal } from "./accounts/AddAccountModal";
import { HealthChips } from "./accounts/HealthChips";
import { CtxMenuForAccount } from "./accounts/useAccountContextMenu";
import { hasUnreportedWindow } from "./accounts/format";

/**
 * How long to wait after a wake before refetching usage.
 *
 * Measured against the live API on 2026-07-25: `resets_at` was still
 * null immediately after the wake call and populated by t+20s. Refetch
 * any sooner and the card repaints the same "—", which reads as the
 * action having failed. 25s buys margin over the observed lag.
 */
const WAKE_REFRESH_DELAY_MS = 25_000;

import {
  useAccountHandlers,
  verifyLiveFor,
} from "./accounts/useAccountHandlers";
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
  const { t } = useTranslation("accounts");
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

  // "Wake windows" — the one action here that spends plan quota, so it
  // is click-only and never batched.
  //
  // `waking` is a Set, not a single uuid: with one slot, two wakes in
  // flight would let the first one's completion clear the busy flag for
  // the second, re-enabling the menu item and inviting another spend.
  // The uuid stays in the Set through the delayed refresh, not just the
  // request, because `needsWake` keeps returning true until usage
  // repaints — so a single-uuid guard released at request-end would
  // still allow repeat clicks for the next ~25 seconds.
  const [waking, setWaking] = useState<ReadonlySet<string>>(new Set());
  const wakeTimers = useRef<number[]>([]);
  // `api.accountWake` can still be in flight when the section unmounts,
  // and its continuation would otherwise schedule a timer *after*
  // cleanup already ran — leaking exactly the timer cleanup exists to
  // prevent. The ref lets the continuation notice it is orphaned.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      // `useUsage` is owned by this section, so a timer that outlives it
      // would call into a dead hook — a wasted IPC round-trip whose
      // result nothing can render.
      wakeTimers.current.forEach(window.clearTimeout);
      wakeTimers.current = [];
    };
  }, []);

  const handleWake = useCallback(
    async (a: AccountSummary) => {
      let started = false;
      setWaking((prev) => {
        if (prev.has(a.uuid)) return prev; // already in flight — no double spend
        started = true;
        return new Set(prev).add(a.uuid);
      });
      if (!started) return;

      const release = () =>
        setWaking((prev) => {
          const next = new Set(prev);
          next.delete(a.uuid);
          return next;
        });

      try {
        const receipt = await api.accountWake(a.uuid);
        pushToast(
          "info",
          t("section.wokeToast", {
            email: receipt.email,
            input: receipt.input_tokens,
            output: receipt.output_tokens,
          }),
        );
        if (!mounted.current) return; // orphaned — nothing to refresh into
        const timer = window.setTimeout(() => {
          // Release only once usage has actually repainted. Releasing
          // when the refresh *starts* leaves a gap where `needsWake` is
          // still true and the menu item is live again — another click,
          // another spend.
          void refreshUsageFor(a.uuid).finally(release);
        }, WAKE_REFRESH_DELAY_MS);
        wakeTimers.current.push(timer);
      } catch (err) {
        pushToast("error", renderError(err, t("section.wakeFailed")));
        release();
      }
    },
    [pushToast, refreshUsageFor, t],
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
      // Stage the query for a lazy-mounting KeysSection (its listener
      // isn't wired until the chunk mounts — a delayed CustomEvent
      // alone dropped on first navigation, audit 2026-07 F4), then
      // dispatch for an already-mounted one. Whichever path consumes
      // the query clears the staged copy.
      setPendingKeysFilter(email);
      onNavigate?.("keys");
      window.dispatchEvent(
        new CustomEvent("cp-keys-filter", { detail: { query: email } }),
      );
    },
    [onNavigate],
  );

  const [showAdd, setShowAdd] = useState(false);
  const [filter, setFilter] = useState("");
  const [ctxMenu, setCtxMenu] = useState<
    | { kind: "row"; x: number; y: number; account: AccountSummary }
    | null
  >(null);

  const { runVerifyAccount, runVerifyAll, handleDesktopSwitch, verify } =
    useAccountHandlers({
      pushToast,
      refresh,
      useDesktop: actions.useDesktop,
    });
  // Denominator for the "Verifying… n/total" label: prefer the count
  // streamed with the first outcome, fall back to the visible account
  // count for the frame before the first event lands.
  const verifyTotal = verify.total || accounts.length;

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.preventDefault();
      setCtxMenu({ kind: "row", x: e.clientX, y: e.clientY, account: a });
    },
    [],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  // Cmd+Shift+C — copy first matching email when a filter is active,
  // else the first account in the list. Shift makes `e.key` report
  // "C", so match case-insensitively (same convention as
  // useShellShortcuts' ⌘⇧L), and respect the shared modal/editable
  // gate so the shortcut never fires over a dialog or while typing.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || !e.shiftKey || e.altKey) return;
      if (e.key !== "c" && e.key !== "C") return;
      if (isShortcutContextBlocked()) return;
      e.preventDefault();
      const target = shown[0];
      if (!target) return;
      void navigator.clipboard
        .writeText(target.email)
        .then(() =>
          pushToast("info", t("section.copiedEmail", { email: target.email })),
        )
        .catch((err) =>
          pushToast("error", renderError(err, t("section.copyFailed"))),
        );
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `shown` is computed below — tracked by accounts/filter deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accounts, filter, pushToast, t]);

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
          pushToast("error", renderError(e, t("section.desktopLaunchFailed")));
        });
      },
      adoptDesktop: (a) => {
        if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
        else void actions.adoptDesktop(a);
      },
    }),
    [handleDesktopSwitch, actions, requestDesktopOverwrite, pushToast, t],
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
            {t("section.loadErrorTitle")}
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-sm)",
              margin: 0,
              textAlign: "center",
            }}
          >
            {t("section.loadErrorBody")}
          </p>
          <Button variant="solid" onClick={() => refresh()}>
            {t("section.retry")}
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
              {t("section.errorDetail")}
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
        label={t("section.loading")}
        style={{ padding: "var(--sp-32)" }}
      />
    );
  }

  return (
    <>
      <ScreenHeader
        title={t("section.title")}
        subtitle={<HealthChips accounts={accounts} />}
        actions={
          <>
            {compact ? (
              <>
                <IconButton
                  glyph={NF.shield}
                  onClick={runVerifyAll}
                  disabled={verify.active}
                  title={
                    verify.active
                      ? t("section.verifyingProgress", {
                          done: verify.done,
                          total: verifyTotal,
                        })
                      : t("section.verifyAllCompactTitle")
                  }
                  aria-label={
                    verify.active
                      ? t("section.verifyingProgressAria", {
                          done: verify.done,
                          total: verifyTotal,
                        })
                      : t("section.verifyAllAria")
                  }
                />
                <IconButton
                  glyph={NF.refresh}
                  onClick={() => {
                    refresh();
                    refreshUsage();
                  }}
                  title={t("section.refreshUsageTitleCompact")}
                  aria-label={t("section.refreshUsage")}
                />
              </>
            ) : (
              <>
                <Button
                  variant="ghost"
                  glyph={NF.shield}
                  glyphColor="var(--fg-muted)"
                  onClick={runVerifyAll}
                  disabled={verify.active}
                  title={
                    verify.active
                      ? t("section.verifyAllBusyTitle")
                      : t("section.verifyAllTitle")
                  }
                >
                  {verify.active
                    ? t("section.verifyingProgress", {
                        done: verify.done,
                        total: verifyTotal,
                      })
                    : t("section.verifyAll")}
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
                        ? formatDateTime(new Date(lastFetchedAt))
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
                  title={t("section.refreshTitle")}
                >
                  {t("section.refreshUsage")}
                </Button>
              </>
            )}
            <Button
              variant="solid"
              glyph={NF.plus}
              onClick={() => setShowAdd(true)}
              title={t("section.addAccountTitle")}
            >
              {t("section.addAccount")}
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
            pushToast("info", t("section.adoptedToast", { email: outcome.email }));
            await refresh();
          } catch (e) {
            pushToast("error", renderError(e, t("section.adoptFailed")));
          }
        }}
        verifyLiveFor={(uuid) => verifyLiveFor(verify, uuid)}
        onRefreshUsage={(a) => refreshUsageFor(a.uuid)}
        onVerifyAccount={runVerifyAccount}
      />

      <AddAccountModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        accounts={accounts}
        onAdded={async () => {
          setShowAdd(false);
          await refresh();
          pushToast("info", t("section.accountAdded"));
        }}
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
          onWake={(a) => void handleWake(a)}
          needsWake={hasUnreportedWindow(
            usage[ctxMenu.account.uuid]?.usage ?? null,
          )}
          wakeBusy={waking.has(ctxMenu.account.uuid)}
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
  if (deltaMs < 30_000) return i18n.t("usageAge.justNow", { ns: "accounts" });
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 1) return i18n.t("usageAge.justNow", { ns: "accounts" });
  if (minutes < 60) {
    return i18n.t("usageAge.minutesAgo", { ns: "accounts", mins: minutes });
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return i18n.t("usageAge.hoursAgo", { ns: "accounts", hours });
  }
  const days = Math.floor(hours / 24);
  return i18n.t("usageAge.daysAgo", { ns: "accounts", days });
}
