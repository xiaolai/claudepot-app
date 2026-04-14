import type { AccountSummary } from "../types";
import { TokenBadge } from "./TokenBadge";

function getDisabledHint(a: AccountSummary, desktop: boolean): string | null {
  if (!a.credentials_healthy) return "Credentials missing — log in to restore";
  if (!desktop) return "Desktop app not detected";
  if (!a.has_desktop_profile) return "No Desktop profile — sign in via Desktop first";
  return null;
}

export function AccountCard({
  account: a,
  desktopAvailable,
  busyKeys,
  anyBusy,
  onUseCli,
  onUseDesktop,
  onLogin,
  onCancelLogin,
  onRemove,
}: {
  account: AccountSummary;
  desktopAvailable: boolean;
  busyKeys: Set<string>;
  anyBusy: boolean;
  onUseCli: () => void;
  onUseDesktop: () => void;
  onLogin: () => void;
  onCancelLogin: () => void;
  onRemove: () => void;
}) {
  const cliBusy = busyKeys.has(`cli-${a.uuid}`);
  const deskBusy = busyKeys.has(`desk-${a.uuid}`);
  const reBusy = busyKeys.has(`re-${a.uuid}`);
  const rmBusy = busyKeys.has(`rm-${a.uuid}`);
  const selfBusy = cliBusy || deskBusy || reBusy || rmBusy;

  const desktopDisabled =
    selfBusy ||
    a.is_desktop_active ||
    !desktopAvailable ||
    !a.has_desktop_profile;

  const hint =
    !a.is_desktop_active && desktopDisabled && !selfBusy
      ? getDisabledHint(a, desktopAvailable)
      : null;

  const deskTitle = !desktopAvailable ? "Desktop not installed"
    : a.is_desktop_active ? "Already active Desktop"
    : !a.has_desktop_profile ? "No Desktop profile yet — sign in via the Desktop app first"
    : "Use for Desktop (quits + relaunches Claude)";

  const active = a.is_cli_active || a.is_desktop_active;

  return (
    <article className={`account ${a.is_cli_active ? "cli-active" : ""} ${a.is_desktop_active ? "desktop-active" : ""}`}
      aria-current={active ? "true" : undefined}>
      <div className="account-main">
        <div className="account-head">
          <h3>{a.email}</h3>
          {a.is_cli_active && <span className="slot-badge cli">CLI</span>}
          {a.is_desktop_active && <span className="slot-badge desktop">Desktop</span>}
          <TokenBadge status={a.token_status} mins={a.token_remaining_mins} />
        </div>
        <div className="account-meta muted">{a.org_name ?? "—"} · {a.subscription_type ?? "—"}</div>
      </div>
      <div className="account-actions">
        {a.credentials_healthy ? (
          <button onClick={onUseCli} disabled={selfBusy || a.is_cli_active}
            title={a.is_cli_active ? "Already active CLI" : "Use for CLI"}>
            {cliBusy ? "…" : a.is_cli_active ? "✓ CLI" : "Use CLI"}
          </button>
        ) : reBusy ? (
          <button onClick={onCancelLogin} className="danger" title="Cancel the in-flight browser login">Cancel login</button>
        ) : (
          <button onClick={onLogin} disabled={selfBusy} className="warn"
            title={`Sign in as ${a.email} — opens the browser, imports credentials.`}>Log in</button>
        )}
        <button onClick={onUseDesktop} disabled={desktopDisabled} title={deskTitle}>
          {deskBusy ? "…" : a.is_desktop_active ? "✓ Desktop" : "Use Desktop"}
        </button>
        <button onClick={onRemove} disabled={selfBusy || anyBusy} className="danger"
          title="Remove account, credentials, and profile">{rmBusy ? "…" : "Remove"}</button>
      </div>
      {hint && <div className="account-hint muted">{hint}</div>}
    </article>
  );
}
