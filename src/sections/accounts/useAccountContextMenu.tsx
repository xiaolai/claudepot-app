import { useMemo } from "react";
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
  return useMemo(() => {
    // `desktop_profile_on_disk` is the disk truth; we prefer it over
    // `has_desktop_profile` (the DB cache) per plan v2 D18. The
    // context menu gates on disk truth so a stale flag can't enable
    // a swap that would immediately fail at `restore()`.
    const hasProfile = a.desktop_profile_on_disk;
    const desktopReason = !status.desktop_installed
      ? "Claude Desktop not installed"
      : !hasProfile
        ? "bind current Desktop session first"
        : a.is_desktop_active
          ? "already active"
          : undefined;
    const adoptDesktopDisabled =
      !status.desktop_installed || !onAdoptDesktop;
    const adoptDesktopReason = !status.desktop_installed
      ? "Claude Desktop not installed"
      : undefined;
    const cliReason = a.is_cli_active
      ? "already active"
      : !a.credentials_healthy
        ? "credentials missing or corrupt"
        : undefined;
    const loginBusy = busyKeys.has(`re-${a.uuid}`);

    return [
      {
        label: "Copy email",
        onClick: () => navigator.clipboard.writeText(a.email),
      },
      // UUID is an internal identifier — dev-mode only (design.md).
      {
        label: "Copy UUID",
        devOnly: true,
        onClick: () => navigator.clipboard.writeText(a.uuid),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: a.is_cli_active ? "Active CLI" : "Set as CLI",
        disabled: a.is_cli_active || !a.credentials_healthy,
        disabledReason: cliReason,
        onClick: () => onSwitchCli(a),
      },
      {
        label: a.is_desktop_active ? "Active Desktop" : "Set as Desktop",
        disabled:
          a.is_desktop_active || !hasProfile || !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktop(a),
      },
      {
        label: "Set as Desktop (don't relaunch)",
        disabled:
          a.is_desktop_active || !hasProfile || !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktopNoLaunch(a),
      },
      {
        label: "Bind current Desktop session",
        disabled: adoptDesktopDisabled,
        disabledReason: adoptDesktopReason,
        onClick: () => onAdoptDesktop?.(a),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: "Verify now",
        disabled: !a.credentials_healthy,
        disabledReason: !a.credentials_healthy
          ? "no credentials to verify"
          : undefined,
        onClick: () => onVerify(a),
      },
      {
        label: "Refresh usage",
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
              label: `Wake windows (~${WAKE_ESTIMATED_TOKENS} tokens)`,
              disabled: !a.credentials_healthy || wakeBusy,
              disabledReason: !a.credentials_healthy
                ? "no credentials"
                : wakeBusy
                  ? "wake in progress"
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
        label: "Launch CC as…",
        devOnly: true,
        disabled: true,
        disabledReason: "use `claudepot cli run` from your shell",
        onClick: () => {},
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: "Log in again…",
        disabled: loginBusy,
        disabledReason: loginBusy ? "login in progress" : undefined,
        onClick: () => onLogin(a),
      },
      { label: "", separator: true, onClick: () => {} },
      {
        label: "Remove",
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
