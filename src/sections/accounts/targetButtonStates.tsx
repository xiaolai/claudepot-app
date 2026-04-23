import type { ComponentProps } from "react";
import type { ContextMenuItem } from "../../components/ContextMenu";
import { NF } from "../../icons";
import { TargetButton } from "../../components/primitives/TargetButton";
import type { AccountSummary, AppStatus } from "../../types";

type TargetButtonProps = ComponentProps<typeof TargetButton>;

export interface CliTargetHandlers {
  switchCli: (a: AccountSummary) => void;
  verify: (a: AccountSummary) => void;
  login: (a: AccountSummary) => void;
}

export interface DesktopTargetHandlers {
  switchDesktop: (a: AccountSummary) => void;
  switchDesktopNoLaunch: (a: AccountSummary) => void;
  launchDesktop: () => void;
  /** Binds the live Desktop session to this account's slot. The caller
   *  is responsible for routing through the overwrite-confirm dialog
   *  when a snapshot already exists. */
  adoptDesktop: (a: AccountSummary) => void;
}

/**
 * Derive TargetButton props for the CLI slot from an account's state.
 *
 *   is_cli_active            → active   (body inert, chevron = Verify · Re-login)
 *   creds healthy, not bound → available (body = Set as CLI)
 *   creds missing / broken   → disabled  (AnomalyBanner carries the reason;
 *                                         chevron exposes Re-login)
 */
export function cliTargetProps(
  a: AccountSummary,
  h: CliTargetHandlers,
): TargetButtonProps {
  const active = a.is_cli_active;
  const healthy = a.credentials_healthy;

  const menu: ContextMenuItem[] = [
    {
      label: "Verify now",
      disabled: !healthy,
      disabledReason: !healthy ? "no credentials to verify" : undefined,
      onClick: () => h.verify(a),
    },
    { label: "Re-login", onClick: () => h.login(a) },
  ];

  const state = active ? "active" : healthy ? "available" : "disabled";
  const primaryTitle = active
    ? `Active CLI — ${a.email}`
    : healthy
      ? `Switch CLI to ${a.email}`
      : "Credentials missing — re-login from the menu";

  return {
    icon: NF.terminal,
    label: "CLI",
    state,
    onPrimary: state === "available" ? () => h.switchCli(a) : undefined,
    primaryTitle,
    menu,
  };
}

/**
 * Derive TargetButton props for the Desktop slot, or `null` when
 * Claude Desktop is not installed (button simply isn't rendered).
 *
 *   is_desktop_active              → active   (Re-launch · Bind again)
 *   profile exists, not active     → available (Set without relaunch ·
 *                                              Bind again)
 *   no profile, Desktop installed  → adopt    (body = Bind current
 *                                              session; no menu)
 *   Desktop not installed          → null
 */
export function desktopTargetProps(
  a: AccountSummary,
  status: AppStatus,
  h: DesktopTargetHandlers,
): TargetButtonProps | null {
  if (!status.desktop_installed) return null;

  if (a.is_desktop_active) {
    return {
      icon: NF.desktop,
      label: "Desktop",
      state: "active",
      primaryTitle: `Active Desktop — ${a.email}`,
      menu: [
        { label: "Re-launch", onClick: h.launchDesktop },
        { label: "Bind again", onClick: () => h.adoptDesktop(a) },
      ],
    };
  }

  if (a.desktop_profile_on_disk) {
    return {
      icon: NF.desktop,
      label: "Desktop",
      state: "available",
      onPrimary: () => h.switchDesktop(a),
      primaryTitle: `Set Desktop to ${a.email}`,
      menu: [
        {
          label: "Set without relaunch",
          onClick: () => h.switchDesktopNoLaunch(a),
        },
        { label: "Bind again", onClick: () => h.adoptDesktop(a) },
      ],
    };
  }

  // No stored snapshot — the only verb is "adopt the currently-live
  // Desktop session into this account's slot". No menu; one click.
  return {
    icon: NF.desktop,
    label: "Adopt",
    state: "adopt",
    onPrimary: () => h.adoptDesktop(a),
    primaryTitle: "Bind the currently-running Desktop session to this account",
  };
}
