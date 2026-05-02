// Global → Memory.
//
// Static-analysis health cards for the user's CLAUDE.md and the
// per-account MEMORY.md index. Surfaces three signals:
//
//   1. Existence — file there at all?
//   2. Bloat — line count + estimated tokens.
//   3. Visibility — physical lines past CC's truncation cutoff
//      (load-bearing: those lines are invisible to the model).
//
// Pure read; no edit affordances. The user's CLAUDE.md is freeform
// and dictionary-typed — claudepot doesn't presume to suggest what
// to cut.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { FileHealth, MemoryHealthReport } from "../../types";

export function MemoryHealthPanel() {
  const [report, setReport] = useState<MemoryHealthReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchReport = useCallback(async () => {
    setLoading(true);
    try {
      const r = await api.memoryHealthGet();
      setReport(r);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchReport();
  }, [fetchReport]);

  return (
    <div
      style={{
        padding: "var(--sp-12) var(--sp-16)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        flex: 1,
        overflow: "auto",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-10)",
          flexWrap: "wrap",
        }}
      >
        <h2
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            color: "var(--fg)",
            margin: 0,
          }}
        >
          Memory health
        </h2>
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          Static analysis of CLAUDE.md and MEMORY.md.
          {report && (
            <>
              {" CC truncates global memory after line "}
              <strong style={{ color: "var(--fg-muted)" }}>
                {report.line_cutoff}
              </strong>
              {"."}
            </>
          )}
        </span>
        <button
          type="button"
          onClick={() => void fetchReport()}
          disabled={loading}
          style={{
            marginLeft: "auto",
            padding: "var(--sp-3) var(--sp-8)",
            fontSize: "var(--fs-2xs)",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-1)",
            color: "var(--fg)",
            cursor: loading ? "default" : "pointer",
            opacity: loading ? 0.6 : 1,
            fontFamily: "inherit",
          }}
        >
          {loading ? "loading…" : "Refresh"}
        </button>
      </div>
      {error && (
        <div
          role="alert"
          style={{
            color: "var(--danger)",
            fontSize: "var(--fs-xs)",
          }}
        >
          {error}
        </div>
      )}
      {!error && report && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns:
              "repeat(auto-fit, minmax(var(--banner-min-width), 1fr))",
            gap: "var(--sp-10)",
          }}
        >
          <FileHealthCard
            title="CLAUDE.md"
            subtitle="Global instructions"
            health={report.claude_md}
            cutoff={report.line_cutoff}
          />
          <FileHealthCard
            title="MEMORY.md"
            subtitle="Per-account memory index"
            health={report.memory_md}
            cutoff={report.line_cutoff}
          />
        </div>
      )}
    </div>
  );
}

function FileHealthCard({
  title,
  subtitle,
  health,
  cutoff,
}: {
  title: string;
  subtitle: string;
  health: FileHealth;
  cutoff: number;
}) {
  const isBlown = health.lines_past_cutoff > 0;
  return (
    <div
      style={{
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderLeft: isBlown
          ? "var(--bw-accent) solid var(--warn)"
          : "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-10) var(--sp-12)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          gap: "var(--sp-8)",
        }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-1)" }}>
          <div
            style={{
              fontSize: "var(--fs-sm)",
              fontWeight: 500,
              color: "var(--fg)",
              fontFamily: "var(--font-mono, inherit)",
            }}
          >
            {title}
          </div>
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
            }}
          >
            {subtitle}
          </div>
        </div>
        <span
          title={health.path}
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            maxWidth: "var(--memory-path-max, tokens.settings.nav.width)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            direction: "rtl",
            textAlign: "right",
          }}
        >
          {health.path}
        </span>
      </div>
      {health.missing ? (
        <div
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            padding: "var(--sp-6) 0",
          }}
        >
          File not present — nothing to audit.
        </div>
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
            gap: "var(--sp-8)",
          }}
        >
          <Stat
            label="Lines"
            value={health.line_count.toLocaleString()}
          />
          <Stat
            label="Est. tokens"
            value={health.est_tokens.toLocaleString()}
            sub={`${(health.char_count / 1024).toFixed(1)} KB`}
          />
          <Stat
            label={`Past line ${cutoff}`}
            value={health.lines_past_cutoff.toLocaleString()}
            sub={
              health.lines_past_cutoff > 0
                ? `${(health.chars_past_cutoff / 1024).toFixed(1)} KB invisible`
                : "all visible"
            }
            tone={health.lines_past_cutoff > 0 ? "warn" : "ok"}
          />
        </div>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: string;
  sub?: string;
  tone?: "ok" | "warn";
}) {
  const valueColor =
    tone === "warn" ? "var(--warn)" : "var(--fg)";
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-1)",
        minWidth: 0,
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontSize: "var(--fs-md)",
          color: valueColor,
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </div>
      {sub && (
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
          }}
        >
          {sub}
        </div>
      )}
    </div>
  );
}
