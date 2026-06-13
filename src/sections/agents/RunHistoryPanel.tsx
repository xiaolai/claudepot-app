import { Fragment, useEffect, useState } from "react";
import { api } from "../../api";
import { Table, Th, Td } from "../../components/primitives";
import type { AgentRunDto, OutputArtifactDto, RunResultDto } from "../../types";
import { ReportViewer } from "./reports/ReportViewer";

interface Props {
  agentId: string;
  /** Bumped from the parent to trigger a re-fetch (e.g. after Run Now). */
  refreshKey: number;
}

function reportArtifact(run: AgentRunDto): OutputArtifactDto | null {
  const arts = run.output_artifacts ?? [];
  return arts.find((a) => a.kind === "report") ?? null;
}

/**
 * True iff the run carries a parsed `result` event, indicating an
 * agent whose `output_format` was `Json` / `StreamJson`. `Text`
 * runs land here with `result === null` — the disclosure is
 * suppressed and the row falls back to exit-code + log paths.
 */
function hasStructuredResult(run: AgentRunDto): boolean {
  return run.result != null;
}

export function RunHistoryPanel({ agentId, refreshKey }: Props) {
  const [runs, setRuns] = useState<AgentRunDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reportPath, setReportPath] = useState<string | null>(null);
  /** Run ids whose structured-result disclosure is currently open. */
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await api.agentsRunsList(agentId, 20);
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
  }, [agentId, refreshKey]);

  function toggleExpanded(runId: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(runId)) {
        next.delete(runId);
      } else {
        next.add(runId);
      }
      return next;
    });
  }

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
    <>
      <Table
        density="compact"
        style={{ fontSize: "var(--fs-2xs)", fontFamily: "var(--ff-mono)" }}
      >
        <thead>
          <tr>
            <Th>status</Th>
            <Th>started</Th>
            <Th>dur</Th>
            <Th>cost</Th>
            <Th>turns</Th>
            <Th>trigger</Th>
            <Th align="right">details</Th>
          </tr>
        </thead>
        <tbody>
          {runs.map((run) => {
            const ok = !run.result?.is_error && run.exit_code === 0;
            const symbol = ok ? "ok" : "ERR";
            const report = reportArtifact(run);
            const structured = hasStructuredResult(run);
            const isOpen = expanded.has(run.id);
            return (
              <Fragment key={run.id}>
                <tr
                  style={{
                    borderTop: "var(--bw-hair) solid var(--line)",
                    color: ok ? "var(--fg-2)" : "var(--danger)",
                  }}
                >
                  <Td>{symbol}</Td>
                  <Td muted>{fmtIso(run.started_at)}</Td>
                  <Td>{fmtDuration(run.duration_ms)}</Td>
                  <Td>{fmtCost(run.result?.total_cost_usd ?? null)}</Td>
                  <Td>{run.result?.num_turns ?? "—"}</Td>
                  <Td muted>{run.trigger_kind}</Td>
                  <Td align="right">
                    <span
                      style={{
                        display: "inline-flex",
                        gap: "var(--sp-8)",
                        justifyContent: "flex-end",
                      }}
                    >
                      {report ? (
                        <button
                          type="button"
                          onClick={() => setReportPath(report.path)}
                          style={reportLinkStyle}
                          aria-label={`Open report for run started ${run.started_at}`}
                        >
                          report
                        </button>
                      ) : null}
                      {structured ? (
                        <button
                          type="button"
                          onClick={() => toggleExpanded(run.id)}
                          style={reportLinkStyle}
                          aria-expanded={isOpen}
                          aria-controls={`run-${run.id}-details`}
                          aria-label={`${isOpen ? "Hide" : "Show"} structured result for run ${run.id}`}
                        >
                          {isOpen ? "hide" : "show"}
                        </button>
                      ) : null}
                      {!structured && !report ? (
                        <span style={{ color: "var(--fg-3)" }}>—</span>
                      ) : null}
                    </span>
                  </Td>
                </tr>
                {structured && isOpen && run.result ? (
                  <tr id={`run-${run.id}-details`}>
                    <td colSpan={7} style={structuredCellStyle}>
                      <StructuredResultPanel result={run.result} />
                    </td>
                  </tr>
                ) : null}
              </Fragment>
            );
          })}
        </tbody>
      </Table>

      <ReportViewer
        path={reportPath}
        onClose={() => setReportPath(null)}
      />
    </>
  );
}

/**
 * Render the structured `RunResult` fields readably. The run's
 * `output_format` was `Json` / `StreamJson`; the orchestrator
 * parsed CC's terminal `result` event into a [`RunResultDto`].
 * Fields that are `null` / empty are filtered out so a sparsely-
 * populated result doesn't ship a row full of `—`s (paper-mono
 * "render-if-nonzero" rule from design.md).
 */
function StructuredResultPanel({ result }: { result: RunResultDto }) {
  const rows: Array<[string, string]> = [];
  if (result.subtype) rows.push(["subtype", result.subtype]);
  if (result.is_error !== null) {
    rows.push(["is_error", result.is_error ? "true" : "false"]);
  }
  if (result.stop_reason) rows.push(["stop_reason", result.stop_reason]);
  if (result.num_turns !== null) {
    rows.push(["num_turns", String(result.num_turns)]);
  }
  if (result.total_cost_usd !== null) {
    rows.push(["total_cost_usd", `$${result.total_cost_usd.toFixed(4)}`]);
  }
  if (result.session_id) rows.push(["session_id", result.session_id]);

  if (rows.length === 0 && result.errors.length === 0) {
    return (
      <div style={{ color: "var(--fg-3)", fontStyle: "italic" }}>
        Structured result was empty.
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      {rows.length > 0 ? (
        <dl
          style={{
            display: "grid",
            gridTemplateColumns: "auto 1fr",
            gap: "var(--sp-4) var(--sp-12)",
            margin: 0,
          }}
        >
          {rows.map(([k, v]) => (
            <div
              key={k}
              style={{ display: "contents" }}
            >
              <dt style={{ color: "var(--fg-3)", fontWeight: 500 }}>{k}</dt>
              {/* `.selectable` (base.css), not inline `userSelect:
                  "text"` — React omits the -webkit- prefix WKWebView
                  reads first, so the inline form never wins over the
                  body opt-out. Same for the error list below. */}
              <dd
                className="selectable"
                style={{
                  margin: 0,
                  color: "var(--fg-2)",
                  wordBreak: "break-all",
                }}
              >
                {v}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
      {result.errors.length > 0 ? (
        <div>
          <div style={{ color: "var(--danger)", fontWeight: 500 }}>errors</div>
          <ul
            className="selectable"
            style={{
              margin: "var(--sp-4) 0 0",
              paddingLeft: "var(--sp-16)",
              color: "var(--danger)",
            }}
          >
            {result.errors.map((e, i) => (
              <li key={i}>{e}</li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

const reportLinkStyle: React.CSSProperties = {
  background: "none",
  border: "none",
  color: "var(--accent)",
  textDecoration: "underline",
  cursor: "pointer",
  font: "inherit",
  padding: 0,
};

const structuredCellStyle: React.CSSProperties = {
  padding: "var(--sp-8) var(--sp-12)",
  background: "var(--bg-raised)",
  borderTop: "var(--bw-hair) solid var(--line)",
  fontFamily: "var(--ff-mono)",
  fontSize: "var(--fs-2xs)",
};

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
