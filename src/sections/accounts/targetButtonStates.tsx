import type { ComponentProps } from "react";
import type { ContextMenuItem } from "../../components/ContextMenu";
import { NF } from "../../icons";
import { TargetButton } from "../../components/primitives/TargetButton";
import { i18n } from "../../lib/i18n";
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
      label: i18n.t("target.verifyNow", { ns: "accounts" }),
      disabled: !healthy,
      disabledReason: !healthy
        ? i18n.t("target.noCredsToVerify", { ns: "accounts" })
        : undefined,
      onClick: () => h.verify(a),
    },
    {
      label: i18n.t("target.relogin", { ns: "accounts" }),
      onClick: () => h.login(a),
    },
  ];

  const state = active ? "active" : healthy ? "available" : "disabled";
  const primaryTitle = active
    ? i18n.t("target.activeCli", { ns: "accounts", email: a.email })
    : healthy
      ? i18n.t("target.switchCli", { ns: "accounts", email: a.email })
      : i18n.t("target.credsMissing", { ns: "accounts" });

  // No inline caption under the button. The CLI button is only
  // `disabled` when `!credentials_healthy`, which is exactly one of the
  // conditions that make `isAnomaly(a)` true — so the card's
  // AnomalyBanner is *always* already showing the full reason + a
  // Re-login button directly below. A second caption here duplicated
  // that signal (design.md: one signal per surface, no status spray)
  // and, being a line taller than the sibling Desktop pill, rode the
  // whole button cluster out of vertical alignment. The banner carries
  // the reason inline; the button's `primaryTitle` tooltip supplements.
  return {
    icon: NF.terminal,
    label: i18n.t("target.cli", { ns: "accounts" }),
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
      label: i18n.t("target.desktop", { ns: "accounts" }),
      state: "active",
      primaryTitle: i18n.t("target.activeDesktop", {
        ns: "accounts",
        email: a.email,
      }),
      menu: [
        {
          label: i18n.t("target.relaunch", { ns: "accounts" }),
          onClick: h.launchDesktop,
        },
        {
          label: i18n.t("target.bindAgain", { ns: "accounts" }),
          onClick: () => h.adoptDesktop(a),
        },
      ],
    };
  }

  if (a.desktop_profile_on_disk) {
    return {
      icon: NF.desktop,
      label: i18n.t("target.desktop", { ns: "accounts" }),
      state: "available",
      onPrimary: () => h.switchDesktop(a),
      primaryTitle: i18n.t("target.setDesktop", {
        ns: "accounts",
        email: a.email,
      }),
      menu: [
        {
          label: i18n.t("target.setNoRelaunch", { ns: "accounts" }),
          onClick: () => h.switchDesktopNoLaunch(a),
        },
        {
          label: i18n.t("target.bindAgain", { ns: "accounts" }),
          onClick: () => h.adoptDesktop(a),
        },
      ],
    };
  }

  // No stored snapshot — the only verb is "adopt the currently-live
  // Desktop session into this account's slot". No menu; one click.
  // The label stays "Desktop" so the column reads as a single target
  // noun across rows; the dashed border (`state: "adopt"`) and the
  // tooltip carry the "this will bind" signal.
  return {
    icon: NF.desktop,
    label: i18n.t("target.desktop", { ns: "accounts" }),
    state: "adopt",
    onPrimary: () => h.adoptDesktop(a),
    primaryTitle: i18n.t("target.bindTooltip", { ns: "accounts" }),
  };
}
