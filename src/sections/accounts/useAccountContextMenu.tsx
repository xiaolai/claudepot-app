import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ContextMenu, type ContextMenuItem } from "../../components/ContextMenu";
import type { AccountSummary, AppStatus } from "../../types";
import { WAKE_ESTIMATED_TOKENS } from "../../types";

interface Args {
  account: AccountSummary;
  status: AppStatus;
  busyKeys: Set<string>;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onSwitchDesktopNoLaunch: (a: AccountSummary) => void;
  onVerify: (a: AccountSummary) => void;
  onRefreshUsageFor: (a: AccountSummary) => void;
  onRefreshUsageAll: () => void;
  onLogin: (a: AccountSummary) => void;
  onRemove: (a: AccountSummary) => void;
  /** "Bind current Desktop session to this account" — Phase 3+. */
  onAdoptDesktop?: (a: AccountSummary) => void;
  /**
   * Spend ~9 tokens to start this account's rate-limit windows so their
   * resets become reportable. Omit to hide the item entirely.
   */
  onWake?: (a: AccountSummary) => void;
  /**
   * True when at least one window has no reset to report. Computed by
   * the caller, which holds the `UsageMap`; passing the boolean rather
   * than the map keeps this hook free of usage-shape knowledge.
   */
  needsWake?: boolean;
  /** A wake is already in flight for this account. */
  wakeBusy?: boolean;
}

/**
 * Assembles the account-row context menu. Extracted from
 * AccountsSection so the section stays under the per-file LOC limit
 * and the menu's disabled-reason rules live next to each other,
 * readable as a decision table.
 */
export function useAccountContextMenu({
  account: a,
  status,
  busyKeys,
  onSwitchCli,
  onSwitchDesktop,
  onSwitchDesktopNoLaunch,
  onVerify,
  onRefreshUsageFor,
  onRefreshUsageAll,
  onLogin,
  onRemove,
  onAdoptDesktop,
  onWake,
  needsWake = false,
  wakeBusy = false,
}: Args): ContextMenuItem[] {
  const { t } = useTranslation("accounts");
  return useMemo(() => {
    // `desktop_profile_on_disk` is the disk truth; we prefer it over
    // `has_desktop_profile` (the DB cache) per plan v2 D18. The
    // context menu gates on disk truth so a stale flag can't enable
    // a swap that would immediately fail at `restore()`.
    const hasProfile = a.desktop_profile_on_disk;
    const desktopReason = !status.desktop_installed
      ? t("menu.desktopNotInstalled")
      : !hasProfile
        ? t("menu.bindFirst")
        : a.is_desktop_active
          ? t("menu.alreadyActive")
          : undefined;
    const adoptDesktopDisabled =
      !status.desktop_installed || !onAdoptDesktop;
    const adoptDesktopReason = !status.desktop_installed
      ? t("menu.desktopNotInstalled")
      : undefined;
    const cliReason = a.is_cli_active
      ? t("menu.alreadyActive")
      : !a.credentials_healthy
        ? t("menu.credsMissingCorrupt")
        : undefined;
    const loginBusy = busyKeys.has(`re-${a.uuid}`);

    return [
      {
        label: t("menu.copyEmail"),
        onClick: () => navigator.clipboard.writeText(a.email),
      },
      // UUID is an internal identifier — dev-mode only (design.md).
      {
        label: t("menu.copyUuid"),
        devOnly: true,
        onClick: () => navigator.clipboard.writeText(a.uuid),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: a.is_cli_active ? t("menu.activeCli") : t("menu.setCli"),
        disabled: a.is_cli_active || !a.credentials_healthy,
        disabledReason: cliReason,
        onClick: () => onSwitchCli(a),
      },
      {
        label: a.is_desktop_active
          ? t("menu.activeDesktop")
          : t("menu.setDesktop"),
        disabled:
          a.is_desktop_active || !hasProfile || !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktop(a),
      },
      {
        label: t("menu.setDesktopNoRelaunch"),
        disabled:
          a.is_desktop_active || !hasProfile || !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktopNoLaunch(a),
      },
      {
        label: t("menu.bindDesktop"),
        disabled: adoptDesktopDisabled,
        disabledReason: adoptDesktopReason,
        onClick: () => onAdoptDesktop?.(a),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: t("menu.verifyNow"),
        disabled: !a.credentials_healthy,
        disabledReason: !a.credentials_healthy
          ? t("menu.noCredsToVerify")
          : undefined,
        onClick: () => onVerify(a),
      },
      {
        label: t("menu.refreshUsage"),
        onClick: () =>
          a.credentials_healthy ? onRefreshUsageFor(a) : onRefreshUsageAll(),
      },
      // Only offered when a window actually has no reset to report.
      // On an account already in use this is a no-op that costs quota,
      // so it stays hidden rather than sitting there enabled-but-useless.
      ...(onWake && needsWake
        ? [
            {
              // The cost is in the label because the GUI has no confirm
              // step — this label IS the pre-spend disclosure.
              label: t("menu.wakeWindows", { tokens: WAKE_ESTIMATED_TOKENS }),
              disabled: !a.credentials_healthy || wakeBusy,
              disabledReason: !a.credentials_healthy
                ? t("menu.noCreds")
                : wakeBusy
                  ? t("menu.wakeInProgress")
                  : undefined,
              onClick: () => onWake(a),
            } satisfies ContextMenuItem,
          ]
        : []),
      { label: "", separator: true, onClick: () => {} },
      // Launch-CC-as needs a new-terminal spawn that varies per OS.
      // Stub behind dev-mode until that Tauri surface lands; devs can
      // use `claudepot cli run <email> claude` from a shell.
      {
        label: t("menu.launchCcAs"),
        devOnly: true,
        disabled: true,
        disabledReason: t("menu.launchCcAsReason", {
          cmd: "`claudepot cli run`",
        }),
        onClick: () => {},
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: t("menu.loginAgain"),
        disabled: loginBusy,
        disabledReason: loginBusy ? t("menu.loginInProgress") : undefined,
        onClick: () => onLogin(a),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: t("menu.remove"),
        danger: true,
        onClick: () => onRemove(a),
      },
    ];
  }, [
    a,
    status,
    busyKeys,
    onSwitchCli,
    onSwitchDesktop,
    onSwitchDesktopNoLaunch,
    onVerify,
    onRefreshUsageFor,
    onRefreshUsageAll,
    onLogin,
    onRemove,
    onAdoptDesktop,
    onWake,
    needsWake,
    wakeBusy,
    t,
  ]);
}

/**
 * Small hook wrapper that turns the menu-item set into a live
 * ContextMenu. Hook calls must live inside a component — keeping the
 * wrapper next to the hook itself avoids scattering the menu logic
 * across two files.
 */
export function CtxMenuForAccount({
  menu,
  status,
  busyKeys,
  onSwitchCli,
  onSwitchDesktop,
  onSwitchDesktopNoLaunch,
  onVerify,
  onRefreshUsageFor,
  onRefreshUsageAll,
  onLogin,
  onRemove,
  onAdoptDesktop,
  onWake,
  needsWake,
  wakeBusy,
  onClose,
}: {
  menu: { x: number; y: number; account: AccountSummary };
  status: AppStatus;
  busyKeys: Set<string>;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onSwitchDesktopNoLaunch: (a: AccountSummary) => void;
  onVerify: (a: AccountSummary) => void;
  onRefreshUsageFor: (a: AccountSummary) => void;
  onRefreshUsageAll: () => void;
  onLogin: (a: AccountSummary) => void;
  onRemove: (a: AccountSummary) => void;
  onAdoptDesktop?: (a: AccountSummary) => void;
  onWake?: (a: AccountSummary) => void;
  needsWake?: boolean;
  wakeBusy?: boolean;
  onClose: () => void;
}) {
  const items = useAccountContextMenu({
    account: menu.account,
    status,
    busyKeys,
    onSwitchCli,
    onSwitchDesktop,
    onSwitchDesktopNoLaunch,
    onVerify,
    onRefreshUsageFor,
    onRefreshUsageAll,
    onLogin,
    onRemove,
    onAdoptDesktop,
    onWake,
    needsWake,
    wakeBusy,
  });
  return (
    <ContextMenu x={menu.x} y={menu.y} items={items} onClose={onClose} />
  );
}
