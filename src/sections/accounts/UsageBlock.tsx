import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { UsageEntry, UsageWindow } from "../../types";
import { formatResetTime, formatResetTooltip } from "./format";

interface UsageBlockProps {
  entry: UsageEntry | null;
  /**
   * True when the card-level AnomalyBanner is already showing an alert
   * for this account (drift/rejected/expired/unhealthy). We suppress
   * the redundant "token expired" / "no credentials" StatusLine in
   * that case so there's only one signal per surface.
   */
  anomalyShown?: boolean;
}

/**
 * Four rate-limit rows — 5h / 7d all / 7d Sonnet / 7d Opus — each
 * with a 20-segment bar, percentage, and reset time. Renders an
 * inline status message when the entry is expired / rate-limited /
 * error instead of the rows.
 */
export function UsageBlock({ entry, anomalyShown }: UsageBlockProps) {
  if (!entry || entry.status === "no_credentials") {
    if (anomalyShown) return null;
    return (
      <StatusLine glyph={NF.info} tone="muted">
        Usage unavailable — no credentials.
      </StatusLine>
    );
  }
  if (entry.status === "expired") {
    if (anomalyShown) return null;
    return (
      <StatusLine glyph={NF.info} tone="muted">
        Usage unavailable — token expired.
      </StatusLine>
    );
  }
  if (entry.status === "rate_limited") {
    return (
      <StatusLine glyph={NF.clock} tone="muted">
        Rate-limited by /api/oauth/usage · retry in{" "}
        {entry.retry_after_secs ?? 60}s
      </StatusLine>
    );
  }
  if (entry.status === "error") {
    return (
      <StatusLine glyph={NF.warn} tone="warn">
        Couldn't fetch usage: {entry.error_detail ?? "unknown error"}
      </StatusLine>
    );
  }

  const usage = entry.usage;
  if (!usage) {
    return (
      <StatusLine glyph={NF.info} tone="muted">
        No usage windows reported.
      </StatusLine>
    );
  }

  // Row set in display order. `tooltip` only populates for the two
  // plan-level rows that aren't self-evident (OAuth apps / cowork).
  const rows: {
    label: string;
    w: UsageWindow;
    emph: boolean;
    tooltip?: string;
  }[] = [
    { label: "5h window", w: usage.five_hour!, emph: true },
    { label: "7d all", w: usage.seven_day!, emph: false },
    { label: "7d Sonnet", w: usage.seven_day_sonnet!, emph: false },
    { label: "7d Opus", w: usage.seven_day_opus!, emph: false },
    {
      label: "7d apps",
      w: usage.seven_day_oauth_apps!,
      emph: false,
      tooltip:
        "Third-party OAuth apps authorized against this account (IDEs, tools, etc).",
    },
    {
      label: "7d cowork",
      w: usage.seven_day_cowork!,
      emph: false,
      tooltip: "Cowork / shared-seat pool usage.",
    },
  ].filter((r) => r.w) as typeof rows;

  return (
    <div style={{ padding: "var(--sp-14) var(--sp-18) var(--sp-12)" }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "baseline",
          marginBottom: "var(--sp-10)",
        }}
      >
        <span className="mono-cap">Rate-limit windows</span>
        {entry.status === "stale" && entry.age_secs != null && (
          <span
            title={`Cached — last fetched ${formatAgeAbsolute(entry.age_secs)}`}
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              fontVariantNumeric: "tabular-nums",
            }}
          >
            <Glyph
              g={NF.clock}
              style={{
                fontSize: "var(--fs-3xs)",
                marginRight: "var(--sp-4)",
              }}
            />
            {formatAgeShort(entry.age_secs)} old
          </span>
        )}
      </div>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-7)",
        }}
      >
        {rows.map((r) => (
          <UsageRow
            key={r.label}
            label={r.label}
            w={r.w}
            emph={r.emph}
            labelTooltip={r.tooltip}
          />
        ))}
      </div>

      {usage.extra_usage && <ExtraUsageRow extra={usage.extra_usage} />}
    </div>
  );
}

/**
 * Extras row. Three visual states:
 *   1. Enabled & billing    → `$12.50 / $50.00 · 25%`  (server utilization preferred)
 *   2. Enabled, no spend    → `$0.00 / $50.00` (no percent)
 *   3. Disabled             → faint "off" marker on the divider line
 */
function ExtraUsageRow({ extra }: { extra: NonNullable<UsageEntry["usage"]>["extra_usage"] }) {
  if (!extra) return null;

  if (!extra.is_enabled) {
    return (
      <div
        title="Extras (overage billing) is disabled for this account. Enable in the Anthropic console if you need headroom past the monthly limit."
        style={{
          marginTop: "var(--sp-12)",
          paddingTop: "var(--sp-10)",
          borderTop: "var(--bw-hair) dashed var(--line)",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "baseline",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
        }}
      >
        <span className="mono-cap">Extra usage</span>
        <span className="mono-cap">off</span>
      </div>
    );
  }

  const used = extra.used_credits ?? 0;
  const limit = extra.monthly_limit ?? 0;
  // Prefer server-side utilization when present — it accounts for
  // rollover, prorated credits, and grace adjustments that a
  // client-side used/limit ratio misses. Fall back to the ratio when
  // the server omits the field.
  const serverPct = extra.utilization;
  const pct =
    serverPct != null
      ? Math.round(serverPct)
      : limit > 0
        ? Math.round((used / limit) * 100)
        : null;
  const high = pct != null && pct >= 80;

  return (
    <div
      style={{
        marginTop: "var(--sp-12)",
        paddingTop: "var(--sp-10)",
        borderTop: "var(--bw-hair) dashed var(--line)",
        display: "flex",
        justifyContent: "space-between",
        alignItems: "baseline",
        fontSize: "var(--fs-xs)",
      }}
    >
      <span className="mono-cap">Extra usage</span>
      <span
        style={{
          fontVariantNumeric: "tabular-nums",
          color: "var(--fg)",
          display: "inline-flex",
          gap: "var(--sp-6)",
          alignItems: "baseline",
        }}
      >
        <b>${used.toFixed(2)}</b>
        <span style={{ color: "var(--fg-faint)" }}>
          / ${limit.toFixed(2)}
        </span>
        {pct != null && used > 0 && (
          <span
            style={{
              color: high ? "var(--warn)" : "var(--fg-muted)",
              fontWeight: 600,
              marginLeft: "var(--sp-4)",
            }}
          >
            {pct}%
          </span>
        )}
      </span>
    </div>
  );
}

function formatAgeShort(ageSecs: number): string {
  if (ageSecs < 60) return `${Math.max(1, Math.round(ageSecs))}s`;
  const mins = Math.round(ageSecs / 60);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.round(mins / 60);
  return `${hrs}h`;
}

function formatAgeAbsolute(ageSecs: number): string {
  const date = new Date(Date.now() - ageSecs * 1000);
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZoneName: "shortOffset",
  }).format(date);
}

function StatusLine({
  glyph,
  tone,
  children,
}: {
  glyph: string;
  tone: "muted" | "warn";
  children: React.ReactNode;
}) {
  const color = tone === "warn" ? "var(--warn)" : "var(--fg-muted)";
  return (
    <div
      style={{
        padding: "var(--sp-16) var(--sp-18)",
        color,
        fontSize: "var(--fs-xs)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
      }}
    >
      <Glyph g={glyph} style={{ fontSize: "var(--fs-xs)" }} />
      {children}
    </div>
  );
}

function UsageRow({
  label,
  w,
  emph,
  labelTooltip,
}: {
  label: string;
  w: UsageWindow;
  emph: boolean;
  labelTooltip?: string;
}) {
  const pct = Math.round(w.utilization);
  const high = pct >= 80;
  const resetTip = formatResetTooltip(w.resets_at);
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "var(--sp-80) 1fr var(--sp-40) var(--sp-72)",
        gap: "var(--sp-10)",
        alignItems: "center",
        fontSize: "var(--fs-xs)",
      }}
    >
      <span
        title={labelTooltip}
        style={{
          color: emph ? "var(--fg)" : "var(--fg-muted)",
          fontWeight: emph ? 600 : 500,
          // Dotted underline signals "hoverable for more info" on the
          // rows that carry it (apps / cowork). Others stay plain.
          textDecoration: labelTooltip
            ? "underline dotted var(--fg-ghost)"
            : undefined,
          textUnderlineOffset: "0.2em",
          cursor: labelTooltip ? "help" : "default",
        }}
      >
        {label}
      </span>
      <SegBar pct={pct} high={high} />
      <span
        style={{
          fontVariantNumeric: "tabular-nums",
          textAlign: "right",
          fontWeight: 600,
          color: high
            ? "var(--warn)"
            : emph
              ? "var(--fg)"
              : "var(--fg-muted)",
        }}
      >
        {pct}%
      </span>
      <span
        title={resetTip}
        style={{
          textAlign: "right",
          color: "var(--fg-faint)",
          fontVariantNumeric: "tabular-nums",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
          cursor: "help",
        }}
      >
        {formatResetTime(w.resets_at)}
      </span>
    </div>
  );
}

function SegBar({ pct, high }: { pct: number; high: boolean }) {
  const segs = 20;
  // At 20 segments each is 5% of the total, so any utilization
  // below 2.5% rounds to zero filled — the bar reads as "no data"
  // instead of "low usage". Floor a non-zero pct to at least one
  // filled segment so the signal survives the low end.
  const raw = Math.round((pct / 100) * segs);
  const filled = pct > 0 ? Math.max(1, raw) : 0;
  return (
    <div
      style={{ display: "flex", gap: "var(--sp-2)", height: "var(--sp-8)" }}
      aria-hidden
    >
      {Array.from({ length: segs }).map((_, i) => (
        <div
          key={i}
          style={{
            flex: 1,
            // Filled segments read as data (muted ink-on-paper), not
            // brand — terracotta is reserved for the primary CTA and
            // selected state. Warn tone kicks in at >=80% so a
            // critical bar still jumps.
            background:
              i < filled
                ? high
                  ? "var(--warn)"
                  : "var(--fg-muted)"
                : "var(--bg-active)",
            borderRadius: "var(--sp-px)",
            opacity: i < filled ? 1 : "var(--opacity-segbar)",
          }}
        />
      ))}
    </div>
  );
}
