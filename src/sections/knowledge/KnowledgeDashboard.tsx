// The Knowledge pane's landing view.
//
// The pane opens on "here is the state of what Claude knows," never on a
// list and never on "N memories stored." Storage was never the
// bottleneck; a real machine holds thousands of indexed exchanges nobody
// reads back. The number that matters is recurrence → 0: did a failure
// class we already learned about happen again?
//
// Four signals, in priority order (see knowledge-base-pane.md §4.1):
//   1. Trust      — the enforced / documented / suspect / proposed mix.
//   2. Coverage   — of N projects, how many carry any curated knowledge.
//   3. Freshness  — the current suspect total (the invalidation moat).
//   4. Recurrence — the headline once Phase 3 lands; a placeholder here.

import { useCallback, useEffect, useMemo, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type {
  LessonCounts,
  ProjectCounts,
  ProjectSummary,
  RecurrenceCounts,
} from "../../api/sharedMemory";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { basename } from "../../lib/paths";
import {
  StatCard,
  TrustBar,
  trustMix,
  trustTotal,
} from "./dashboard-primitives";
import type { TrustMix } from "./dashboard-primitives";

/** A merged coverage row: a project, its session count, and its trust
 *  mix. Either half can be absent (a project with memories but no indexed
 *  sessions, or vice-versa), so both counts default to zero. */
interface CoverageRow {
  projectPath: string;
  sessionCount: number;
  counts: LessonCounts;
  mix: TrustMix;
  curated: number;
}

const ZERO_COUNTS: LessonCounts = {
  proposed: 0,
  accepted: 0,
  rejected: 0,
  suspect: 0,
  enforced: 0,
};

export function KnowledgeDashboard({
  onOpenProject,
}: {
  onOpenProject: (projectPath: string) => void;
}) {
  const [rollup, setRollup] = useState<LessonCounts | null>(null);
  const [rows, setRows] = useState<CoverageRow[]>([]);
  const [recurrence, setRecurrence] = useState<RecurrenceCounts | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const [counts, byProject, projects, rec] = await Promise.all([
        sharedMemoryApi.lessonCounts(),
        sharedMemoryApi.lessonCountsByProject(),
        sharedMemoryApi.listProjects(),
        sharedMemoryApi.recurrenceCounts(),
      ]);
      setRollup(counts);
      setRows(mergeCoverage(byProject, projects));
      setRecurrence(rec);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const coverage = useMemo(() => {
    const withKnowledge = rows.filter((r) => r.curated > 0).length;
    return { withKnowledge, total: rows.length };
  }, [rows]);

  const rollupMix = rollup ? trustMix(rollup) : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)" }}>
      {err && (
        <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-base)" }}>
          {err}
        </div>
      )}

      {/* The four signals. Recurrence leads (headline); "stored" never
          appears as a hero — it lives as faint secondary text below. */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(11rem, 1fr))",
          gap: "var(--sp-12)",
        }}
      >
        <StatCard
          label="Recurrence"
          value={recurrence?.confirmed_window ?? 0}
          tone={recurrence && recurrence.confirmed_window > 0 ? "warn" : "good"}
          hint={
            recurrence && recurrence.pending > 0
              ? `${recurrence.pending} awaiting confirmation in Review`
              : `known failures that happened again (${recurrence?.window_days ?? 30}d)`
          }
          emphasis
        />
        <StatCard
          label="Suspect"
          value={rollup?.suspect ?? 0}
          tone="warn"
          hint="lessons whose code moved — re-review"
        />
        <StatCard
          label="Enforced"
          value={rollup?.enforced ?? 0}
          tone="good"
          hint="compiled into a check that fails the build"
        />
        <StatCard
          label="Coverage"
          value={`${coverage.withKnowledge} / ${coverage.total}`}
          tone="neutral"
          hint="projects with any curated knowledge"
        />
      </div>

      {/* Trust signal: the roll-up mix across every project. */}
      {rollupMix && trustTotal(rollupMix) > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
          <SectionLabel>Across all projects</SectionLabel>
          <TrustBar mix={rollupMix} height={10} showLegend />
        </div>
      )}

      {/* Coverage grid: most sessions, least curated first — the projects
          worth harvesting float to the top. */}
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        <SectionLabel>Projects</SectionLabel>
        {!loading && rows.length === 0 ? (
          <EmptyDashboard />
        ) : (
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
            {rows.map((row) => (
              <CoverageRowItem key={row.projectPath} row={row} onOpen={onOpenProject} />
            ))}
          </ul>
        )}
      </div>

      {/* "N stored" only ever appears here, faint and last — never a
          hero. It is context, not the point. */}
      {rollup && (
        <p style={{ margin: 0, fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
          {rollup.accepted + rollup.proposed + rollup.suspect} items curated across{" "}
          {rows.length} project{rows.length === 1 ? "" : "s"}.
        </p>
      )}
    </div>
  );
}

// ─── one project row ─────────────────────────────────────────────

function CoverageRowItem({
  row,
  onOpen,
}: {
  row: CoverageRow;
  onOpen: (projectPath: string) => void;
}) {
  const name = basename(row.projectPath);
  return (
    <li>
      <button
        type="button"
        className="pm-focus"
        onClick={() => onOpen(row.projectPath)}
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(0, 1fr) minmax(8rem, 14rem)",
          alignItems: "center",
          gap: "var(--sp-16)",
          width: "100%",
          textAlign: "left",
          border: "var(--sp-px) solid var(--line)",
          borderRadius: "var(--r-2)",
          padding: "var(--sp-12) var(--sp-16)",
          background: "var(--bg-raised)",
          color: "var(--fg)",
          font: "inherit",
          cursor: "pointer",
        }}
      >
        <span style={{ minWidth: 0 }}>
          <span
            style={{
              display: "block",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontWeight: 500,
            }}
            title={row.projectPath}
          >
            {name}
          </span>
          <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
            {row.sessionCount} session{row.sessionCount === 1 ? "" : "s"}
            {row.curated > 0
              ? ` · ${row.curated} curated`
              : " · nothing curated yet"}
          </span>
        </span>
        <TrustBar mix={row.mix} />
      </button>
    </li>
  );
}

function EmptyDashboard() {
  return (
    <div
      style={{
        border: "var(--sp-px) dashed var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-24)",
        textAlign: "center",
        color: "var(--fg-muted)",
      }}
    >
      <p style={{ margin: 0 }}>No projects with indexed sessions yet.</p>
      <p style={{ margin: "var(--sp-8) 0 0", fontSize: "var(--fs-sm)" }}>
        Harvest lessons from your sessions: <code>claudepot lesson harvest</code>
      </p>
    </div>
  );
}

// ─── merge ───────────────────────────────────────────────────────

/** Join per-project review counts with session counts, keyed by project
 *  path. The union of both keysets: a project can have memories but no
 *  indexed sessions (odd but possible) or sessions but no curated
 *  knowledge (the interesting harvest target). Sort surfaces the latter:
 *  uncurated projects first, then by session count descending — "most
 *  sessions, least curated" (knowledge-base-pane.md §4.3). */
export function mergeCoverage(
  byProject: ProjectCounts[],
  projects: ProjectSummary[],
): CoverageRow[] {
  const sessions = new Map<string, number>();
  for (const p of projects) sessions.set(p.project_path, p.session_count);

  const countsByPath = new Map<string, LessonCounts>();
  for (const c of byProject) countsByPath.set(c.project_path, c.counts);

  const paths = new Set<string>([...sessions.keys(), ...countsByPath.keys()]);
  const rows: CoverageRow[] = [];
  for (const projectPath of paths) {
    const counts = countsByPath.get(projectPath) ?? ZERO_COUNTS;
    const mix = trustMix(counts);
    rows.push({
      projectPath,
      sessionCount: sessions.get(projectPath) ?? 0,
      counts,
      mix,
      curated: trustTotal(mix),
    });
  }

  rows.sort((a, b) => {
    // Uncurated (curated === 0) before curated, so a busy-but-unmined
    // project is the first thing the eye lands on.
    const aUncurated = a.curated === 0 ? 0 : 1;
    const bUncurated = b.curated === 0 ? 0 : 1;
    if (aUncurated !== bUncurated) return aUncurated - bUncurated;
    if (a.sessionCount !== b.sessionCount) return b.sessionCount - a.sessionCount;
    return a.projectPath.localeCompare(b.projectPath);
  });
  return rows;
}
