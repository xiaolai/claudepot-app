import { useEffect, useState } from "react";
import { api } from "../../api";
import type { AutomationRunDto, OutputArtifactDto } from "../../types";
import { ReportViewer } from "./reports/ReportViewer";

interface Props {
  automationId: string;
  /** Bumped from the parent to trigger a re-fetch (e.g. after Run Now). */
  refreshKey: number;
}

function reportArtifact(run: AutomationRunDto): OutputArtifactDto | null {
  const arts = run.output_artifacts ?? [];
  return arts.find((a) => a.kind === "report") ?? null;
}

export function RunHistoryPanel({ automationId, refreshKey }: Props) {
  const [runs, setRuns] = useState<AutomationRunDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reportPath, setReportPath] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await api.automationsRunsList(automationId, 20);
        if (!cancelled) {
          setRuns(list);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [automationId, refreshKey]);

  if (error) {
    return (
      <div style={{ color: "var(--danger)", fontSize: "var(--fs-xs)" }}>
        {error}
      </div>
    );
  }
  if (runs === null) {
    return (
      <div style={{ color: "var(--fg-3)", fontSize: "var(--fs-xs)" }}>
        Loading runs…
      </div>
    );
  }
  if (runs.length === 0) {
    return (
      <div style={{ color: "var(--fg-3)", fontSize: "var(--fs-xs)" }}>
        No runs yet.
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        fontSize: "var(--fs-2xs)",
        fontFamily: "var(--ff-mono)",
      }}
    >
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "auto 1fr auto auto auto auto auto",
          gap: "var(--sp-8)",
          color: "var(--fg-3)",
          paddingBottom: "var(--sp-4)",
          borderBottom: "var(--bw-hair) solid var(--line)",
        }}
      >
        <span>status</span>
        <span>started</span>
        <span>dur</span>
        <span>cost</span>
        <span>turns</span>
        <span>trigger</span>
        <span>report</span>
      </div>
      {runs.map((run) => {
        const ok = !run.result?.is_error && run.exit_code === 0;
        const symbol = ok ? "ok" : "ERR";
        const report = reportArtifact(run);
        return (
          <div
            key={run.id}
            style={{
              display: "grid",
              gridTemplateColumns: "auto 1fr auto auto auto auto auto",
              gap: "var(--sp-8)",
              padding: "var(--sp-2) 0",
              color: ok ? "var(--fg-2)" : "var(--danger)",
              alignItems: "center",
            }}
          >
            <span style={{ width: "var(--sp-32)" }}>{symbol}</span>
            <span style={{ color: "var(--fg-2)" }}>
              {fmtIso(run.started_at)}
            </span>
            <span>{fmtDuration(run.duration_ms)}</span>
            <span>{fmtCost(run.result?.total_cost_usd ?? null)}</span>
            <span>{run.result?.num_turns ?? "—"}</span>
            <span style={{ color: "var(--fg-3)" }}>{run.trigger_kind}</span>
            <span>
              {report ? (
                <button
                  type="button"
                  onClick={() => setReportPath(report.path)}
                  style={{
                    background: "none",
                    border: "none",
                    color: "var(--accent)",
                    textDecoration: "underline",
                    cursor: "pointer",
                    font: "inherit",
                    padding: 0,
                  }}
                >
                  report
                </button>
              ) : (
                <span style={{ color: "var(--fg-3)" }}>—</span>
              )}
            </span>
          </div>
        );
      })}

      <ReportViewer
        path={reportPath}
        onClose={() => setReportPath(null)}
      />
    </div>
  );
}

function fmtIso(iso: string): string {
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})/.exec(iso);
  return m ? `${m[1]} ${m[2]}` : iso;
}

function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 60_000)}m`;
}

function fmtCost(usd: number | null): string {
  if (usd === null || usd === undefined) return "—";
  return `$${usd.toFixed(3)}`;
}
