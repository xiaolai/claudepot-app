import { useEffect, useMemo, useRef, useState } from "react";
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

  return <RepoFilterStripInner {...{ groups, activeRepo, onChange }} />;
}

/**
 * Inner component that owns the scroll-strip refs + effects. Split
 * out of the public `RepoFilterStrip` so the `if (groups.length < 2)`
 * guard above keeps the hooks from mounting (and immediately
 * unmounting) when only one repo exists — a full-tree re-render after
 * the filter guard tripped would otherwise violate hook order.
 */
function RepoFilterStripInner({
  groups,
  activeRepo,
  onChange,
}: {
  groups: RepositoryGroup[];
  activeRepo: string | null;
  onChange: (repoId: string | null) => void;
}) {
  // Sort busy repos first so the most-used projects sit near the
  // scroll origin. The "All repos" pill is always rendered first and
  // doesn't participate in the sort. Caller's array is not mutated —
  // sort on a shallow copy so React dev-mode's strict frozen props
  // don't trip over us.
  const sortedGroups = useMemo(
    () => [...groups].sort((a, b) => b.sessions.length - a.sessions.length),
    [groups],
  );

  const scrollerRef = useRef<HTMLDivElement | null>(null);
  // Scroll-position edges drive the fade on each side so the mask
  // only appears when there is actually content past that edge.
  // Drawing a fade against a solid, unscrolled edge would visually
  // imply "there's more here" when there isn't.
  const [edges, setEdges] = useState<{ left: boolean; right: boolean }>({
    left: false,
    right: false,
  });
  useEffect(() => {
    const root = scrollerRef.current;
    if (!root) return;
    const update = () => {
      const canLeft = root.scrollLeft > 1;
      const canRight =
        root.scrollLeft + root.clientWidth < root.scrollWidth - 1;
      setEdges((prev) =>
        prev.left === canLeft && prev.right === canRight
          ? prev
          : { left: canLeft, right: canRight },
      );
    };
    update();
    root.addEventListener("scroll", update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(root);
    return () => {
      root.removeEventListener("scroll", update);
      ro.disconnect();
    };
  }, [sortedGroups.length]);

  // Keep the active pill in view after an external selection change
  // (e.g. cross-section deep-link). `scrollIntoView` on a pill that
  // is already visible is a no-op, so this is safe to run every time
  // `activeRepo` changes.
  useEffect(() => {
    const root = scrollerRef.current;
    if (!root) return;
    const activeEl = root.querySelector<HTMLElement>(
      '[role="tab"][aria-selected="true"]',
    );
    activeEl?.scrollIntoView({
      behavior: "smooth",
      inline: "nearest",
      block: "nearest",
    });
  }, [activeRepo]);

  // Mask: each edge shows a 16px fade only when content extends past
  // that edge. Building the gradient inline lets us drop either stop
  // cleanly — a dead `transparent 0, <opaque> 0` stop would still
  // render a hairline in some engines. The mask uses `--mask-opaque`
  // (semantic token for "fully shown"), not a visual color.
  const opaque = "var(--mask-opaque)";
  const maskLeft = edges.left
    ? `transparent 0, ${opaque} var(--sp-16)`
    : `${opaque} 0`;
  const maskRight = edges.right
    ? `${opaque} calc(100% - var(--sp-16)), transparent 100%`
    : `${opaque} 100%`;
  const maskImage = `linear-gradient(to right, ${maskLeft}, ${maskRight})`;

  return (
    <div
      role="tablist"
      aria-label="Repository filter"
      data-testid="repo-filter-strip"
      className="scrollbar-none"
      ref={scrollerRef}
      onWheel={(e) => {
        // On macOS trackpads, horizontal intent still arrives on
        // deltaX; on regular wheels, the user scrolls deltaY and
        // expects the horizontal strip to move. Translate vertical
        // wheel intent into horizontal scroll when there's no
        // intentional horizontal component.
        if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
          const el = e.currentTarget;
          el.scrollLeft += e.deltaY;
        }
      }}
      style={{
        display: "flex",
        flexWrap: "nowrap",
        alignItems: "center",
        gap: "var(--sp-4)",
        padding: "var(--sp-6) var(--sp-32) var(--sp-10)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg)",
        overflowX: "auto",
        overflowY: "hidden",
        scrollBehavior: "smooth",
        scrollbarWidth: "none",
        msOverflowStyle: "none",
        // Dynamic edge fade — only apply the fade on sides that have
        // more content past the viewport. See the `edges` state
        // above.
        maskImage,
        WebkitMaskImage: maskImage,
      }}
    >
      <RepoPill
        active={activeRepo == null}
        label="All repos"
        count={groups.reduce((n, g) => n + g.sessions.length, 0)}
        onClick={() => onChange(null)}
      />
      {sortedGroups.map((g) => {
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
        flexShrink: 0,
        whiteSpace: "nowrap",
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
