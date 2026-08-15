import { useTranslation } from "react-i18next";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import { verifyKind } from "./verifyStatus";
import { relTime } from "./format";
import type { VerifyLive } from "./useAccountHandlers";

interface HealthFooterProps {
  account: AccountSummary;
  /** Live state during a "Verify all" run: `"verifying"` shows the
   *  pulse; an outcome flips the verify cell to that result before the
   *  terminal refresh persists it; undefined → use `verify_status`. */
  verifyLive?: VerifyLive;
}

/**
 * 2×2 grid at the bottom of the card: verify state, token state,
 * last CLI switch, last Desktop switch. All values are tabular-nums
 * so cards align.
 */
export function HealthFooter({ account: a, verifyLive }: HealthFooterProps) {
  const { t } = useTranslation("accounts");
  // A run is actively checking THIS account — show a neutral pulse
  // instead of the stale prior result so the user sees work happening.
  const isVerifying = verifyLive === "verifying";
  // Once an outcome streams in (verifyLive is an outcome, not
  // "verifying"), flip to it immediately; otherwise the persisted
  // status. `effectiveStatus` is only read when !isVerifying.
  const effectiveStatus =
    verifyLive && verifyLive !== "verifying" ? verifyLive : a.verify_status;

  // Accent (--accent-ink / terracotta) is reserved for the primary CTA
  // and selected state per design.md. A green check plus --ok tone
  // reads as "healthy" without over-claiming brand attention.
  const kind = verifyKind(effectiveStatus);
  const verifyTone = isVerifying
    ? "var(--fg-muted)"
    : kind === "ok"
      ? "var(--ok)"
      : kind === "drift" || kind === "needsLogin"
        ? "var(--warn)"
        : "var(--fg-faint)";

  // Identity lives in the card header — don't repeat the email here.
  // Drift keeps the target email because that's the load-bearing
  // signal (slot is misfiled to another email). For "ok" we append a
  // freshness suffix so a 3-day-old "verified" doesn't read as
  // reassurance.
  // One exhaustive switch rather than a ternary chain. The chain was
  // six deep by the time `signed_out` was added, and a status with no
  // branch of its own fell through to "not yet verified" — which reads
  // as "we haven't looked", the opposite of a terminal state.
  const verifyLabel = isVerifying
    ? t("footer.verifying")
    : verifyFooterLabel();

  function verifyFooterLabel(): string {
    switch (effectiveStatus) {
      case "ok":
        // "verified" alone reads as reassurance for a check that may be
        // days old, so the freshness suffix is part of the label.
        return a.verified_at
          ? t("footer.verifiedAgo", { ago: relTime(a.verified_at) })
          : t("footer.verified");
      case "drift":
        // Keeps the target email — that IS the load-bearing signal.
        return t("footer.drift", { email: a.verified_email ?? "?" });
      case "rejected":
        return t("footer.tokenRejected");
      case "signed_out":
        return t("footer.signedOut");
      case "network_error":
        return t("footer.profileUnreachable");
      case "never":
        return t("footer.notYetVerified");
      default:
        return unknownStatusLabel(effectiveStatus);
    }
  }

  /** Exhaustiveness guard — see `verifyStatus.ts`. */
  function unknownStatusLabel(status: never): string {
    void status;
    return t("footer.notYetVerified");
  }

  return (
    <div
      style={{
        marginTop: "auto",
        padding: "var(--sp-10) var(--sp-18)",
        borderTop: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        rowGap: "var(--sp-6)",
        columnGap: "var(--sp-12)",
        fontSize: "var(--fs-2xs)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      <Cell tone={verifyTone} weight={500}>
        {isVerifying ? (
          // Pulsing dot, not a glyph — a steady spinner-substitute that
          // reads as "in progress" without color carrying the meaning
          // alone (the word "verifying…" sits beside it).
          // `.cp-pulse-dot` honors prefers-reduced-motion (static dot).
          <span
            className="cp-pulse-dot"
            aria-hidden="true"
            style={{ background: "var(--fg-muted)" }}
          />
        ) : (
          <Glyph
            g={effectiveStatus === "ok" ? NF.check : NF.warn}
            color={verifyTone}
            style={{ fontSize: "var(--fs-2xs)" }}
          />
        )}
        <span
          aria-live={isVerifying ? "polite" : undefined}
          style={{
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {verifyLabel}
        </span>
      </Cell>

      <Cell tone="var(--fg-muted)">
        <Glyph
          g={NF.key}
          color="var(--fg-faint)"
          style={{ fontSize: "var(--fs-2xs)" }}
        />
        {/* token_status already carries the remaining-time phrase
            ("valid (6h 11m remaining)"); appending "· 371m left"
            was the same data in a different unit. Line stays on
            one row and reads cleanly. */}
        <span
          style={{
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {t("footer.token", { status: a.token_status })}
        </span>
      </Cell>

      {/* Empty switch values render as nothing — an em-dash is noise,
          not information. Cells drop cleanly when the account has
          never been bound to that target. */}
      {a.last_cli_switch && (
        <Cell tone="var(--fg-muted)">
          <Glyph
            g={NF.terminal}
            color="var(--fg-faint)"
            style={{ fontSize: "var(--fs-2xs)" }}
          />
          <span>{t("footer.cliSwitch", { ago: relTime(a.last_cli_switch) })}</span>
        </Cell>
      )}

      {a.last_desktop_switch && (
        <Cell tone="var(--fg-muted)">
          <Glyph
            g={NF.users}
            color="var(--fg-faint)"
            style={{ fontSize: "var(--fs-2xs)" }}
          />
          <span>
            {t("footer.desktopSwitch", { ago: relTime(a.last_desktop_switch) })}
          </span>
        </Cell>
      )}
    </div>
  );
}

function Cell({
  tone,
  weight,
  children,
}: {
  tone: string;
  weight?: number;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        color: tone,
        fontWeight: weight,
        overflow: "hidden",
      }}
    >
      {children}
    </div>
  );
}
