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

import { useCallback, useEffect, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type {
  LessonCounts,
  ProjectCounts,
  ProjectSummary,
  RecurrenceCounts,
} from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { basename } from "../../lib/paths";
import { toUserError } from "../../lib/errors";
import type { QueueTarget } from "../SharedMemorySection";
import {
  StatCard,
  TrustBar,
  trustMix,
  trustTotal,
} from "./dashboard-primitives";
import type { StatCardProps, TrustMix } from "./dashboard-primitives";

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
  onOpenReview,
}: {
  onOpenProject: (projectPath: string) => void;
  onOpenReview: (queue?: QueueTarget) => void;
}) {
  const [rollup, setRollup] = useState<LessonCounts | null>(null);
  const [rows, setRows] = useState<CoverageRow[]>([]);
  const [recurrence, setRecurrence] = useState<RecurrenceCounts | null>(null);
  const [loading, setLoading] = useState(true);
  // `err` = every call failed (nothing to show). `partial` = some failed but
  // we render what loaded. Distinguishing them is the whole point: a failed
  // load must never masquerade as a healthy, empty, all-zeros dashboard.
  const [err, setErr] = useState<string | null>(null);
  const [partial, setPartial] = useState<string | null>(null);
  // Which specific calls failed — so a lead that ASSERTS a fact (cold-start,
  // "no failure has recurred") is only shown when the call backing that fact
  // actually loaded, not when its failure coerced the value to zero.
  const [countsFailed, setCountsFailed] = useState(false);
  const [recFailed, setRecFailed] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setErr(null);
    setPartial(null);
    // allSettled, not all: one failing call (e.g. the newest recurrence
    // table) must not blank the other three that succeeded.
    const [countsR, byProjectR, projectsR, recR] = await Promise.allSettled([
      sharedMemoryApi.lessonCounts(),
      sharedMemoryApi.lessonCountsByProject(),
      sharedMemoryApi.listProjects(),
      sharedMemoryApi.recurrenceCounts(),
    ]);
    if (countsR.status === "fulfilled") setRollup(countsR.value);
    if (recR.status === "fulfilled") setRecurrence(recR.value);
    setCountsFailed(countsR.status === "rejected");
    setRecFailed(recR.status === "rejected");
    setRows(
      mergeCoverage(
        byProjectR.status === "fulfilled" ? byProjectR.value : [],
        projectsR.status === "fulfilled" ? projectsR.value : [],
      ),
    );

    const results = [countsR, byProjectR, projectsR, recR];
    const failures = results.filter(
      (r): r is PromiseRejectedResult => r.status === "rejected",
    );
    if (failures.length === results.length) {
      setErr(toUserError(failures[0]!.reason));
    } else if (failures.length > 0) {
      setPartial("Part of the dashboard couldn't load — showing what's available.");
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const rollupMix = rollup ? trustMix(rollup) : null;

  // Total knowledge ever recorded — distinguishes cold-start (nothing
  // harvested) from caught-up (harvested, nothing pending). A green "all
  // clear" is only honest in the latter.
  const totalKnowledge = rollup
    ? rollup.proposed + rollup.accepted + rollup.rejected + rollup.suspect
    : 0;
  const proposed = rollup?.proposed ?? 0;
  const suspect = rollup?.suspect ?? 0;
  const enforced = rollup?.enforced ?? 0;
  const documented = rollup ? Math.max(0, rollup.accepted - rollup.enforced) : 0;
  const pendingRec = recurrence?.pending ?? 0;
  const confirmedRec = recurrence?.confirmed_window ?? 0;
  const windowDays = recurrence?.window_days ?? 30;

  // Every hero is open work that clicks to the action that clears it
  // (design.md anti-vanity rule). Recurrence leads — it is the metric the
  // whole compiler exists to drive to zero — then stale, then intake.
  const attention: StatCardProps[] = [];
  if (pendingRec > 0)
    attention.push({
      label: pendingRec === 1 ? "Recurrence" : "Recurrences",
      value: pendingRec,
      tone: "warn",
      hint: "already-learned failures seen again — confirm in Review",
      onClick: () => onOpenReview("proposed"),
    });
  if (suspect > 0)
    attention.push({
      label: "Suspect",
      value: suspect,
      tone: "warn",
      hint: "accepted lessons whose code moved — re-review",
      onClick: () => onOpenReview("suspect"),
    });
  if (proposed > 0)
    attention.push({
      label: "Proposals",
      value: proposed,
      tone: "accent",
      hint: "waiting on your yes / no",
      onClick: () => onOpenReview("proposed"),
    });

  // Secondary context — the trust scoreboard, deliberately NOT a hero here
  // (Review's Gazette owns the scoreboard framing). Render-if-nonzero.
  const context: string[] = [];
  if (enforced > 0) context.push(`${enforced} enforced`);
  if (documented > 0) context.push(`${documented} documented`);
  if (confirmedRec > 0)
    context.push(
      `${confirmedRec} confirmed repeat${confirmedRec === 1 ? "" : "s"} (${windowDays}d)`,
    );

  // ── every call failed: never render a fake-healthy zero dashboard ──
  if (err) {
    return (
      <div
        role="alert"
        style={{ display: "flex", flexDirection: "column", gap: "var(--sp-12)", alignItems: "flex-start" }}
      >
        <p style={{ margin: 0, color: "var(--danger)", fontSize: "var(--fs-base)" }}>{err}</p>
        <Button variant="subtle" onClick={() => void refresh()} disabled={loading}>
          {loading ? "Retrying…" : "Retry"}
        </Button>
      </div>
    );
  }

  // ── first load, nothing yet: hold rather than flash all-zero green ──
  if (loading && rollup === null && recurrence === null && rows.length === 0) {
    return <p style={{ margin: 0, color: "var(--fg-muted)" }}>Loading…</p>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)" }}>
      {partial && (
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)", fontSize: "var(--fs-sm)", color: "var(--warn)" }}>
          <span>{partial}</span>
          <Button variant="ghost" onClick={() => void refresh()} disabled={loading}>
            {loading ? "…" : "Retry"}
          </Button>
        </div>
      )}

      {/* The lead: open work first; otherwise an honest empty/quiet state —
          never a green zero that a cold start or a failure could fake. When
          a specific call failed, its zero can't back a claim: if counts
          failed we suppress the cold-start/caught-up leads entirely (the
          partial banner + Retry above is the signal), and if only recurrence
          failed we drop the "no failure has recurred" clause. */}
      {attention.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
          <SectionLabel>Needs attention</SectionLabel>
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(11rem, 1fr))",
              gap: "var(--sp-12)",
            }}
          >
            {attention.map((s, i) => (
              <StatCard key={s.label} {...s} emphasis={i === 0} />
            ))}
          </div>
        </div>
      ) : countsFailed ? null : totalKnowledge === 0 && rows.length === 0 ? (
        <EmptyDashboard />
      ) : totalKnowledge === 0 ? (
        <div style={{ ...calloutStyle(), color: "var(--fg-muted)" }}>
          <p style={{ margin: 0 }}>
            No lessons yet — harvest a busy project below to begin.
          </p>
        </div>
      ) : (
        <div style={calloutStyle()}>
          <p style={{ margin: 0, color: "var(--ok)", fontWeight: 500 }}>All caught up.</p>
          <p style={{ margin: "var(--sp-6) 0 0", fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
            {recFailed
              ? "Nothing awaiting review. Recurrence status couldn't be loaded — retry above."
              : "Nothing awaiting review, and no known failure has recurred."}
          </p>
        </div>
      )}

      {/* Secondary context line — the scoreboard, not a hero. */}
      {context.length > 0 && (
        <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
          {context.join(" · ")}
        </p>
      )}

      {/* Trust signal: the roll-up mix across every project. */}
      {rollupMix && trustTotal(rollupMix) > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
          <SectionLabel>Across all projects</SectionLabel>
          <TrustBar mix={rollupMix} height={10} showLegend />
        </div>
      )}

      {/* Coverage grid: most sessions, least curated first — the projects
          worth harvesting float to the top. */}
      {rows.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
          <SectionLabel>Projects</SectionLabel>
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
            {rows.map((row) => (
              <CoverageRowItem key={row.projectPath} row={row} onOpen={onOpenProject} />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** Shared box for the lead's non-attention callouts (cold-start / caught-up). */
function calloutStyle(): React.CSSProperties {
  return {
    border: "var(--sp-px) solid var(--line)",
    borderRadius: "var(--r-3)",
    padding: "var(--sp-16)",
    background: "var(--bg-raised)",
  };
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
              ? ` · ${row.curated} lesson${row.curated === 1 ? "" : "s"}`
              : " · no lessons yet"}
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
