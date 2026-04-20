import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import { relTime } from "./format";

interface HealthFooterProps {
  account: AccountSummary;
}

/**
 * 2×2 grid at the bottom of the card: verify state, token state,
 * last CLI switch, last Desktop switch. All values are tabular-nums
 * so cards align.
 */
export function HealthFooter({ account: a }: HealthFooterProps) {
  const verifyTone =
    a.verify_status === "ok"
      ? "var(--accent-ink)"
      : a.verify_status === "drift" || a.verify_status === "rejected"
        ? "var(--warn)"
        : "var(--fg-faint)";

  // Identity lives in the card header — don't repeat the email here.
  // Drift keeps the target email because that's the load-bearing
  // signal (slot is misfiled to another email).
  const verifyLabel =
    a.verify_status === "ok"
      ? "verified"
      : a.verify_status === "drift"
        ? `drift → ${a.verified_email ?? "?"}`
        : a.verify_status === "rejected"
          ? "token rejected"
          : a.verify_status === "network_error"
            ? "profile unreachable"
            : "not yet verified";

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
        <Glyph
          g={a.verify_status === "ok" ? NF.check : NF.warn}
          color={verifyTone}
          style={{ fontSize: "var(--fs-2xs)" }}
        />
        <span
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
          token {a.token_status}
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
          <span>CLI switch {relTime(a.last_cli_switch)}</span>
        </Cell>
      )}

      {a.last_desktop_switch && (
        <Cell tone="var(--fg-muted)">
          <Glyph
            g={NF.users}
            color="var(--fg-faint)"
            style={{ fontSize: "var(--fs-2xs)" }}
          />
          <span>Desktop switch {relTime(a.last_desktop_switch)}</span>
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
