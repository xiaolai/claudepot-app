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
 *   broken     rejected | expired | bad blob   (danger — creds are dead, re-login required)
 *
 * Order matters: broken wins over drift wins over unverified wins over ok,
 * so a single anomalous account lands in its most severe bucket.
 */
function categorize(a: AccountSummary): Bucket {
  if (!a.credentials_healthy) return "broken";
  if (a.verify_status === "rejected") return "broken";
  if (a.token_status === "expired") return "broken";
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
  glyph: NfIcon;
  tone: string;
  count: number;
  title: string;
  /** Label for screen readers — paired with the numeric count. */
  aria: string;
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
  if (accounts.length === 0) {
    return (
      <span style={{ color: "var(--fg-muted)" }}>
        No accounts registered yet.
      </span>
    );
  }

  const buckets = count(accounts);

  // Total count is always shown. Health chips are render-if-nonzero
  // with order "positive first, then severity ascending" so healthy
  // counts read before warnings.
  const chips: ChipDef[] = [
    {
      glyph: NF.users,
      tone: "var(--fg-muted)",
      count: accounts.length,
      title: `${accounts.length} account${accounts.length === 1 ? "" : "s"} total`,
      aria: "accounts total",
    },
  ];

  if (buckets.ok > 0) {
    chips.push({
      glyph: NF.check,
      tone: "var(--ok)",
      count: buckets.ok,
      title: `${buckets.ok} verified recently`,
      aria: "verified",
    });
  }
  if (buckets.unverified > 0) {
    chips.push({
      glyph: NF.circle,
      tone: "var(--fg-faint)",
      count: buckets.unverified,
      title: `${buckets.unverified} not yet verified`,
      aria: "unverified",
    });
  }
  if (buckets.drift > 0) {
    chips.push({
      glyph: NF.warn,
      tone: "var(--warn)",
      count: buckets.drift,
      title: `${buckets.drift} drift — slot misfiled, re-login or remove`,
      aria: "drift",
    });
  }
  if (buckets.broken > 0) {
    chips.push({
      glyph: NF.ban,
      tone: "var(--warn)",
      count: buckets.broken,
      title: `${buckets.broken} broken — credentials rejected, expired, or unreadable`,
      aria: "broken",
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
      aria-label="Account health summary"
    >
      {chips.map((chip) => (
        <span
          key={chip.aria}
          role="listitem"
          title={chip.title}
          aria-label={`${chip.count} ${chip.aria}`}
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
