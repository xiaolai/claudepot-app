import { useMemo } from "react";
import { ContextMenu, type ContextMenuItem } from "../../components/ContextMenu";
import type { AccountSummary, AppStatus } from "../../types";

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
}: Args): ContextMenuItem[] {
  return useMemo(() => {
    const desktopReason = !status.desktop_installed
      ? "Claude Desktop not installed"
      : !a.has_desktop_profile
        ? "sign in via Desktop app first"
        : a.is_desktop_active
          ? "already active"
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
          a.is_desktop_active ||
          !a.has_desktop_profile ||
          !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktop(a),
      },
      {
        label: "Set as Desktop (don't relaunch)",
        disabled:
          a.is_desktop_active ||
          !a.has_desktop_profile ||
          !status.desktop_installed,
        disabledReason: desktopReason,
        onClick: () => onSwitchDesktopNoLaunch(a),
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
  });
  return (
    <ContextMenu x={menu.x} y={menu.y} items={items} onClose={onClose} />
  );
}
