import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";

interface AnomalyBannerProps {
  account: AccountSummary;
  onRelogin?: () => void;
  disabled?: boolean;
}

/**
 * In-card warning strip. Triggers on drift / rejected / expired /
 * unhealthy — see `isAnomaly(a)`. Always exposes a "Re-login" button
 * that fires `onRelogin` (wired to `api.accountLogin`).
 */
export function AnomalyBanner({
  account,
  onRelogin,
  disabled,
}: AnomalyBannerProps) {
  const copy = anomalyCopy(account);
  if (!copy) return null;

  return (
    <div
      role="alert"
      style={{
        padding: "var(--sp-10) var(--sp-18)",
        background: "color-mix(in oklch, var(--warn) 12%, transparent)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-10)",
      }}
    >
      <Glyph
        g={NF.warn}
        color="var(--warn)"
        style={{ fontSize: "var(--fs-base)", marginTop: "var(--sp-2)" }}
      />
      <div
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: "var(--fs-xs)",
        }}
      >
        <div
          style={{
            fontWeight: 600,
            color: "var(--fg)",
            marginBottom: "var(--sp-2)",
          }}
        >
          {copy.title}
        </div>
        <div style={{ color: "var(--fg-muted)" }}>{copy.detail}</div>
      </div>
      <button
        type="button"
        onClick={onRelogin}
        disabled={disabled}
        title="Open a browser OAuth flow and import the result"
        style={{
          padding: "var(--sp-3) var(--sp-8)",
          fontSize: "var(--fs-xs)",
          background: "var(--bg-raised)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-1)",
          color: "var(--fg)",
          cursor: disabled ? "not-allowed" : "pointer",
          whiteSpace: "nowrap",
          fontWeight: 500,
          opacity: disabled ? "var(--opacity-dimmed)" : 1,
        }}
      >
        Re-login
      </button>
    </div>
  );
}

/**
 * True when the account is in a state that needs human attention.
 * The four cases match `anomalyCopy` below one-to-one.
 */
export function isAnomaly(a: AccountSummary): boolean {
  return (
    a.drift ||
    a.verify_status === "rejected" ||
    a.token_status === "expired" ||
    !a.credentials_healthy
  );
}

function anomalyCopy(
  a: AccountSummary,
): { title: string; detail: React.ReactNode } | null {
  if (a.drift) {
    return {
      title: "Wrong account on this slot",
      detail: (
        <>
          The credentials saved here actually belong to{" "}
          <strong style={{ color: "var(--fg)" }}>
            {a.verified_email || "another account"}
          </strong>
          . Log in again to fix this, or remove this account.
        </>
      ),
    };
  }
  if (a.verify_status === "rejected") {
    return {
      title: "Server rejected the saved login",
      detail: "The stored login is no longer valid — log in again to fix.",
    };
  }
  if (a.token_status === "expired") {
    return {
      title: "Session expired",
      detail: "Log in again to refresh usage for this account.",
    };
  }
  if (!a.credentials_healthy) {
    return {
      title: "Saved login is missing or broken",
      detail:
        "The stored credential file couldn't be read. Log in again or remove.",
    };
  }
  return null;
}
