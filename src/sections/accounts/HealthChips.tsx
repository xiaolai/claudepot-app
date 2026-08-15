import { useTranslation } from "react-i18next";
import { Glyph } from "../../components/primitives/Glyph";
import type { NfIcon } from "../../icons";
import { NF } from "../../icons";
import type { AccountSummary } from "../../types";
import { verifyKind } from "./verifyStatus";

type Bucket = "ok" | "unverified" | "drift" | "broken";

/**
 * Collapse the raw account states into 4 user-facing buckets:
 *
 *   ok         verify_status === "ok"           (green check)
 *   unverified "never" or "network_error"       (grey circle — we just don't know)
 *   drift      "drift"                          (warn — slot misfiled, worth attention)
 *   broken     rejected | signed_out | bad blob (danger — re-login required)
 *
 * Order matters: broken wins over drift wins over unverified wins over ok,
 * so a single anomalous account lands in its most severe bucket.
 *
 * `signed_out` (Claude Code cleared its own credentials) is "broken",
 * not "unverified". The distinction is the whole point of the state:
 * "unverified" means we could not check, and the honest response is to
 * wait; "broken" means we checked and only a re-login recovers it. A
 * terminal state sitting in the wait-and-see bucket is what left users
 * watching for a self-heal that could not come.
 *
 * `token_status === "expired"` is intentionally NOT a "broken" signal:
 * the verify pass auto-refreshes via the OAuth refresh_token within a
 * second of the next focus/refresh tick, and a stuck refresh flips the
 * row to "rejected" — which IS broken. Counting locally-expired tokens
 * here false-alarms during the cold-paint window before verify runs.
 */
function categorize(a: AccountSummary): Bucket {
  // An unreadable blob is "broken" regardless of what the last verify
  // pass recorded — there is nothing left to have an opinion about.
  if (!a.credentials_healthy) return "broken";
  switch (verifyKind(a.verify_status)) {
    case "needsLogin":
      return "broken";
    case "drift":
      return "drift";
    case "ok":
      return "ok";
    case "unknown":
      return "unverified";
  }
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
