import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { RepositoryGroup, SessionRow } from "../../types";

/**
 * Horizontal pill strip listing every repository that owns at least
 * one session. Clicking a pill toggles the active repo filter.
 * Worktree children collapse into the same pill as their repo root —
 * the Rust `session_worktree::group_by_repo` pipeline did that work.
 *
 * The parent owns both `groups` and `activeRepo` state so the filter
 * survives re-renders. `activeRepo` identifies a group by
 * `groupId(group)` — basename-based labels aren't unique (two repos
 * named `docs/` would collide), so we key on the canonical `repo_root`
 * when present, falling back to the label string for the "no repo"
 * bucket.
 */
export function RepoFilterStrip({
  groups,
  activeRepo,
  onChange,
}: {
  groups: RepositoryGroup[] | null;
  activeRepo: string | null;
  onChange: (repoId: string | null) => void;
}) {
  if (!groups || groups.length < 2) return null;

  return (
    <div
      role="tablist"
      aria-label="Repository filter"
      data-testid="repo-filter-strip"
      style={{
        display: "flex",
        flexWrap: "wrap",
        gap: "var(--sp-4)",
        padding: "var(--sp-6) var(--sp-32) var(--sp-10)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
      }}
    >
      <RepoPill
        active={activeRepo == null}
        label="All repos"
        count={groups.reduce((n, g) => n + g.sessions.length, 0)}
        onClick={() => onChange(null)}
      />
      {groups.map((g) => {
        const id = groupId(g);
        return (
          <RepoPill
            key={id}
            active={activeRepo === id}
            label={g.label}
            count={g.sessions.length}
            worktrees={g.worktree_paths.length}
            branches={g.branches}
            onClick={() => onChange(activeRepo === id ? null : id)}
          />
        );
      })}
    </div>
  );
}

/**
 * Stable identifier for a group. Two repos can share a basename label
 * (two different folders both called `docs`), so we key off the
 * canonical `repo_root` when git located one. The "no repo" bucket is
 * unique, so its label suffices.
 */
function groupId(g: RepositoryGroup): string {
  return g.repo_root ?? g.label;
}

function RepoPill({
  active,
  label,
  count,
  worktrees,
  branches,
  onClick,
}: {
  active: boolean;
  label: string;
  count: number;
  worktrees?: number;
  branches?: string[];
  onClick: () => void;
}) {
  const title =
    branches && branches.length > 0
      ? `${label} — ${count} session(s), ${worktrees ?? 1} worktree(s), branches: ${branches.join(", ")}`
      : `${label} — ${count} session(s)`;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      title={title}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
        padding: "var(--sp-2) var(--sp-8)",
        fontSize: "var(--fs-xs)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        background: active ? "var(--accent-soft)" : "transparent",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        cursor: "pointer",
        fontFamily: "inherit",
      }}
    >
      <Glyph g={NF.git} style={{ fontSize: "var(--fs-2xs)" }} />
      <span>{label}</span>
      <span style={{ color: "var(--fg-ghost)" }}>{count}</span>
      {worktrees && worktrees > 1 && (
        <span
          style={{
            color: "var(--fg-ghost)",
            fontSize: "var(--fs-3xs)",
            letterSpacing: "var(--ls-wide)",
          }}
        >
          · {worktrees}wt
        </span>
      )}
    </button>
  );
}

/**
 * Pure filter helper: narrow a session list to the chosen repo. Safe
 * to call with `null` groups (returns all sessions) or `null` label
 * (also returns all).
 */
export function filterSessionsByRepo(
  sessions: SessionRow[],
  groups: RepositoryGroup[] | null,
  repoId: string | null,
): SessionRow[] {
  if (!repoId || !groups) return sessions;
  const g = groups.find((g) => groupId(g) === repoId);
  if (!g) return sessions;
  const wt = new Set(g.worktree_paths);
  return sessions.filter((s) => wt.has(s.project_path));
}
