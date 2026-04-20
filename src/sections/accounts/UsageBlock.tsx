import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { UsageEntry, UsageWindow } from "../../types";
import { formatResetTime } from "./format";

interface UsageBlockProps {
  entry: UsageEntry | null;
}

/**
 * Four rate-limit rows — 5h / 7d all / 7d Sonnet / 7d Opus — each
 * with a 20-segment bar, percentage, and reset time. Renders an
 * inline status message when the entry is expired / rate-limited /
 * error instead of the rows.
 */
export function UsageBlock({ entry }: UsageBlockProps) {
  if (!entry || entry.status === "no_credentials") {
    return (
      <StatusLine glyph={NF.info} tone="muted">
        Usage unavailable — no credentials.
      </StatusLine>
    );
  }
  if (entry.status === "expired") {
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

  const rows: { label: string; w: UsageWindow; emph: boolean }[] = [
    { label: "5h window", w: usage.five_hour!, emph: true },
    { label: "7d all", w: usage.seven_day!, emph: false },
    { label: "7d Sonnet", w: usage.seven_day_sonnet!, emph: false },
    { label: "7d Opus", w: usage.seven_day_opus!, emph: false },
  ].filter((r) => r.w) as { label: string; w: UsageWindow; emph: boolean }[];

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
        {entry.status === "stale" &&
          entry.age_secs != null &&
          entry.age_secs > 60 && (
            <span
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
            >
              <Glyph
                g={NF.clock}
                style={{
                  fontSize: "var(--fs-3xs)",
                  marginRight: "var(--sp-4)",
                }}
              />
              as of {Math.round(entry.age_secs / 60)}m ago
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
          <UsageRow key={r.label} label={r.label} w={r.w} emph={r.emph} />
        ))}
      </div>

      {usage.extra_usage?.is_enabled && (
        <div
          style={{
            marginTop: "var(--sp-12)",
            paddingTop: "var(--sp-10)",
            borderTop: "var(--bw-hair) dashed var(--line)",
            display: "flex",
            justifyContent: "space-between",
            fontSize: "var(--fs-xs)",
          }}
        >
          <span className="mono-cap">Extra usage</span>
          <span
            style={{
              fontVariantNumeric: "tabular-nums",
              color: "var(--fg)",
            }}
          >
            <b>
              ${(usage.extra_usage.used_credits ?? 0).toFixed(2)}
            </b>
            <span style={{ color: "var(--fg-faint)" }}>
              {" / $"}
              {(usage.extra_usage.monthly_limit ?? 0).toFixed(2)}
            </span>
          </span>
        </div>
      )}
    </div>
  );
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
}: {
  label: string;
  w: UsageWindow;
  emph: boolean;
}) {
  const pct = Math.round(w.utilization);
  const high = pct >= 80;
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
        style={{
          color: emph ? "var(--fg)" : "var(--fg-muted)",
          fontWeight: emph ? 600 : 500,
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
        style={{
          textAlign: "right",
          color: "var(--fg-faint)",
          fontVariantNumeric: "tabular-nums",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {formatResetTime(w.resets_at)}
      </span>
    </div>
  );
}

function SegBar({ pct, high }: { pct: number; high: boolean }) {
  const segs = 20;
  const filled = Math.round((pct / 100) * segs);
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
