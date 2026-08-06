import { useTranslation } from "react-i18next";
import { Glyph } from "../../components/primitives/Glyph";
import type { NfIcon } from "../../icons";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";

type Bucket = "ok" | "unverified" | "drift" | "broken";

/**
 * Collapse the 7 raw account states into 4 user-facing buckets:
 *
 *   ok         verify_status === "ok"          (green check)
 *   unverified "never" or "network_error"      (grey circle — we just don't know)
 *   drift      "drift"                         (warn — slot misfiled, worth attention)
 *   broken     rejected | bad blob             (danger — creds are dead, re-login required)
 *
 * Order matters: broken wins over drift wins over unverified wins over ok,
 * so a single anomalous account lands in its most severe bucket.
 *
 * `token_status === "expired"` is intentionally NOT a "broken" signal:
 * the verify pass auto-refreshes via the OAuth refresh_token within a
 * second of the next focus/refresh tick, and a stuck refresh flips the
 * row to "rejected" — which IS broken. Counting locally-expired tokens
 * here false-alarms during the cold-paint window before verify runs.
 */
function categorize(a: AccountSummary): Bucket {
  if (!a.credentials_healthy) return "broken";
  if (a.verify_status === "rejected") return "broken";
  if (a.verify_status === "drift") return "drift";
  if (a.verify_status === "ok") return "ok";
  // Covers "never" and "network_error".
  return "unverified";
}

function count(
  accounts: AccountSummary[],
): Record<Bucket, number> {
  const c: Record<Bucket, number> = {
    ok: 0,
    unverified: 0,
    drift: 0,
    broken: 0,
  };
  for (const a of accounts) c[categorize(a)] += 1;
  return c;
}

interface ChipDef {
  /** Stable id — React key; never translated. */
  id: string;
  glyph: NfIcon;
  tone: string;
  count: number;
  title: string;
  /** Full label for screen readers, count included. */
  ariaLabel: string;
}

interface Props {
  accounts: AccountSummary[];
}

/**
 * Header subtitle chips: total account count + up to 4 health-state
 * chips (render-if-nonzero). Sits in the ScreenHeader subtitle slot,
 * replacing the earlier prose ("3 accounts · 1 needs attention").
 */
export function HealthChips({ accounts }: Props) {
  const { t } = useTranslation("accounts");
  if (accounts.length === 0) {
    return (
      <span style={{ color: "var(--fg-muted)" }}>
        {t("chips.none")}
      </span>
    );
  }

  const buckets = count(accounts);

  // Total count is always shown. Health chips are render-if-nonzero
  // with order "positive first, then severity ascending" so healthy
  // counts read before warnings.
  const chips: ChipDef[] = [
    {
      id: "total",
      glyph: NF.users,
      tone: "var(--fg-muted)",
      count: accounts.length,
      title: t("chips.totalTitle", { count: accounts.length }),
      ariaLabel: t("chips.totalAria", { n: accounts.length }),
    },
  ];

  if (buckets.ok > 0) {
    chips.push({
      id: "verified",
      glyph: NF.check,
      tone: "var(--ok)",
      count: buckets.ok,
      title: t("chips.verifiedTitle", { n: buckets.ok }),
      ariaLabel: t("chips.verifiedAria", { n: buckets.ok }),
    });
  }
  if (buckets.unverified > 0) {
    chips.push({
      id: "unverified",
      glyph: NF.circle,
      tone: "var(--fg-faint)",
      count: buckets.unverified,
      title: t("chips.unverifiedTitle", { n: buckets.unverified }),
      ariaLabel: t("chips.unverifiedAria", { n: buckets.unverified }),
    });
  }
  if (buckets.drift > 0) {
    chips.push({
      id: "drift",
      glyph: NF.warn,
      tone: "var(--warn)",
      count: buckets.drift,
      title: t("chips.driftTitle", { n: buckets.drift }),
      ariaLabel: t("chips.driftAria", { n: buckets.drift }),
    });
  }
  if (buckets.broken > 0) {
    chips.push({
      id: "broken",
      glyph: NF.ban,
      tone: "var(--warn)",
      count: buckets.broken,
      title: t("chips.brokenTitle", { n: buckets.broken }),
      ariaLabel: t("chips.brokenAria", { n: buckets.broken }),
    });
  }

  return (
    <div
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-12)",
        fontSize: "var(--fs-xs)",
        fontVariantNumeric: "tabular-nums",
      }}
      role="list"
      aria-label={t("chips.summaryAria")}
    >
      {chips.map((chip) => (
        <span
          key={chip.id}
          role="listitem"
          title={chip.title}
          aria-label={chip.ariaLabel}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-4)",
            color: chip.tone,
          }}
        >
          <Glyph g={chip.glyph} />
          <span style={{ fontWeight: 600 }}>{chip.count}</span>
        </span>
      ))}
    </div>
  );
}
