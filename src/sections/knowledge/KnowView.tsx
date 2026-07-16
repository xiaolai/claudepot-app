// The Know view — the curated base, project-first.
//
// Replaces the two flat lists (Memories, Decisions) and folds in the
// previously-invisible Evidence table. One project-grouped browser that
// shows state, provenance, and cross-links: the actual knowledge base,
// not a bigger library. The pipeline (Review) is the intake; manual
// authoring is a deliberately secondary affordance here.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { Decision, Evidence, LessonRow } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { Glyph } from "../../components/primitives/Glyph";
import { CopyButton } from "../../components/CopyButton";
import { NF } from "../../icons";
import { basename } from "../../lib/paths";
import { AddMemoryForm } from "./AddMemoryForm";
import { TrustBar } from "./dashboard-primitives";
import type { TrustMix } from "./dashboard-primitives";
import { toUserError } from "../../lib/errors";
import {
  isEnforced,
  KnowItemCard,
  memoryStateBadge,
  decisionStateBadge,
} from "./knowledge-items";
import type { KnowItem } from "./knowledge-items";

const GLOBAL_KEY = "(global)";
/** Per-type row cap. A list at this length is flagged as truncated. */
const LIMIT = 500;

type StateFilter =
  | "all"
  | "proposed"
  | "accepted"
  | "enforced"
  | "suspect"
  | "active"
  | "superseded";

type KindFilter =
  | "all"
  | "fact"
  | "preference"
  | "pattern"
  | "constraint"
  | "summary"
  | "decision"
  | "evidence";

export function KnowView({
  initialProjectFilter = null,
  initialMemoryId = null,
  onReview,
}: {
  /** Deep-link from the Dashboard: pre-filter to one project. */
  initialProjectFilter?: string | null;
  /** Deep-link from Review to the matched lesson. */
  initialMemoryId?: string | null;
  /** Route to the Review tab, optionally targeting a sub-queue. */
  onReview: (queue?: "proposed" | "suspect") => void;
}) {
  const [memories, setMemories] = useState<LessonRow[]>([]);
  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [evidence, setEvidence] = useState<Evidence[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [partial, setPartial] = useState<string | null>(null);
  const [truncated, setTruncated] = useState<string[]>([]);

  const [stateFilter, setStateFilter] = useState<StateFilter>("all");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [query, setQuery] = useState("");
  const [projectFilter, setProjectFilter] = useState<string>(
    initialProjectFilter ?? "all",
  );
  const [showAdd, setShowAdd] = useState(false);

  // The deep-link carrier drives the project filter: a fresh jump
  // (Dashboard → project row) sets it, and a cleared carrier (a plain
  // Know-tab click after a drill-down) resets it back to "all". Without the
  // reset, the Know tab would stay silently filtered to a project the user
  // drilled into once — an invisible filter that makes the base look empty.
  useEffect(() => {
    setProjectFilter(initialProjectFilter ?? "all");
  }, [initialProjectFilter]);

  const refresh = useCallback(async () => {
    setLoading(true);
    setErr(null);
    setPartial(null);
    // allSettled: one failing list (e.g. the newest evidence table) must not
    // blank the memories + decisions that loaded fine.
    const [mR, dR, eR] = await Promise.allSettled([
      sharedMemoryApi.lessonList({ state: "all", limit: LIMIT }),
      sharedMemoryApi.listDecisions({ limit: LIMIT }),
      sharedMemoryApi.listEvidence({ limit: LIMIT }),
    ]);
    // Rejected lessons are a settled "no" — they live in Review's history,
    // never in the curated base. Archived decisions leave the base too
    // (mirroring archived memories, which the backend already drops);
    // superseded stays, to show a decision's evolution.
    if (mR.status === "fulfilled")
      setMemories(mR.value.filter((row) => row.review_state !== "rejected"));
    if (dR.status === "fulfilled")
      setDecisions(dR.value.filter((x) => x.status !== "archived"));
    if (eR.status === "fulfilled") setEvidence(eR.value);

    // A list at the cap is almost certainly hiding rows — say so rather than
    // present a truncated base as complete.
    setTruncated(
      [
        mR.status === "fulfilled" && mR.value.length >= LIMIT ? "memories" : null,
        dR.status === "fulfilled" && dR.value.length >= LIMIT ? "decisions" : null,
        eR.status === "fulfilled" && eR.value.length >= LIMIT ? "evidence" : null,
      ].filter((x): x is string => x !== null),
    );

    const results = [mR, dR, eR];
    const failures = results.filter(
      (r): r is PromiseRejectedResult => r.status === "rejected",
    );
    if (failures.length === results.length) setErr(toUserError(failures[0]!.reason));
    else if (failures.length > 0)
      setPartial("Part of the base couldn't load — showing what's available.");
    setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const items = useMemo<KnowItem[]>(() => {
    const out: KnowItem[] = [];
    for (const row of memories)
      out.push({
        type: "memory",
        id: row.id,
        projectPath: row.project_path,
        createdAtMs: row.created_at_ms,
        row,
      });
    for (const row of decisions)
      out.push({
        type: "decision",
        id: row.id,
        projectPath: row.project_path,
        createdAtMs: row.created_at_ms,
        row,
      });
    for (const row of evidence)
      out.push({
        type: "evidence",
        id: row.id,
        projectPath: row.project_path,
        createdAtMs: row.created_at_ms,
        row,
      });
    return out;
  }, [memories, decisions, evidence]);

  const projects = useMemo(() => {
    const set = new Set<string>();
    for (const it of items) set.add(it.projectPath ?? GLOBAL_KEY);
    return Array.from(set).sort();
  }, [items]);

  const filtered = useMemo(
    () =>
      items.filter(
        (it) =>
          matchesProject(it, projectFilter) &&
          matchesKind(it, kindFilter) &&
          matchesState(it, stateFilter) &&
          matchesText(it, query),
      ),
    [items, projectFilter, kindFilter, stateFilter, query],
  );

  const groups = useMemo(() => groupByProject(filtered), [filtered]);

  // ── keyboard navigation (j/k move, enter opens provenance) ──
  //
  // Collapse state is lifted here (not local to ProjectGroup) so the
  // cursor knows which items are actually visible: an item inside a
  // collapsed group is not a j/k stop.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [cursor, setCursor] = useState(0);
  const [openProvenance, setOpenProvenance] = useState<string | null>(null);
  const cardRefs = useRef(new Map<string, HTMLDivElement>());
  // The deep-linked memory is sought exactly once (when it first appears in
  // the visible set). Without this one-shot latch it would re-fire on every
  // `visible` change and yank the cursor back on every later filter/search,
  // defeating the cursor-reset effect below.
  const memorySeekDone = useRef(false);

  const visible = useMemo(
    () =>
      groups
        .filter((g) => !collapsed.has(g.key))
        .flatMap((g) => g.items),
    [groups, collapsed],
  );

  // Keep the cursor in range as filters / collapse change the list.
  useEffect(() => {
    setCursor((i) => Math.min(i, Math.max(0, visible.length - 1)));
  }, [visible.length]);

  // A filter change re-anchors the cursor to the top rather than keeping a
  // now-meaningless index — otherwise the focus ring jumps to an unrelated
  // card whenever the visible set changes identity without changing length.
  useEffect(() => {
    setCursor(0);
  }, [projectFilter, kindFilter, stateFilter, query]);

  // Review → recurrence → Know carries the matched memory selection across
  // the tab boundary. The cursor is visual rather than DOM focus, so the
  // initiating tab control keeps focus and keyboard navigation remains
  // available.
  useEffect(() => {
    // The latch resets only when the deep-link clears (id → null). Today that
    // always happens between two memory deep-links, because `openMemoryInKnow`
    // switches tabs and KnowView is conditionally unmounted, so its ref is
    // fresh on the next mount. If a future refactor keeps KnowView always
    // mounted, a direct A→B relink (no null between) would keep this latched
    // and never seek B — reset on any id change, not just on clear.
    if (!initialMemoryId) {
      memorySeekDone.current = false;
      return;
    }
    if (memorySeekDone.current) return;
    const index = visible.findIndex(
      (it) => it.type === "memory" && it.id === initialMemoryId,
    );
    if (index >= 0) {
      setCursor(index);
      memorySeekDone.current = true;
    }
  }, [initialMemoryId, visible]);

  const toggleProvenance = useCallback((key: string) => {
    setOpenProvenance((cur) => (cur === key ? null : key));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const tag = el?.tagName;
      // Never hijack typing in the filter controls.
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      // `Math.max(0, …)` keeps the cursor at 0 (not -1) when the list is
      // empty — e.g. every group collapsed.
      if (e.key === "j")
        setCursor((i) => Math.min(i + 1, Math.max(0, visible.length - 1)));
      else if (e.key === "k") setCursor((i) => Math.max(i - 1, 0));
      else if (e.key === "Enter") {
        // Let a focused button/link handle its own Enter.
        if (tag === "BUTTON" || tag === "A") return;
        const it = visible[cursor];
        if (it && it.type === "memory" && it.row.origin_file_path) {
          e.preventDefault();
          toggleProvenance(`${it.type}:${it.id}`);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, cursor, toggleProvenance]);

  // Scroll the focused card into view as the cursor moves.
  useEffect(() => {
    const it = visible[cursor];
    if (!it) return;
    cardRefs.current
      .get(`${it.type}:${it.id}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [cursor, visible]);

  const focusedKey =
    visible[cursor] != null
      ? `${visible[cursor]!.type}:${visible[cursor]!.id}`
      : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
      <FilterBar
        stateFilter={stateFilter}
        kindFilter={kindFilter}
        query={query}
        projectFilter={projectFilter}
        projects={projects}
        addOpen={showAdd}
        onState={setStateFilter}
        onKind={setKindFilter}
        onQuery={setQuery}
        onProject={setProjectFilter}
        onToggleAdd={() => setShowAdd((v) => !v)}
      />

      {showAdd && (
        <AddMemoryForm
          defaultProject={
            projectFilter !== "all" && projectFilter !== GLOBAL_KEY
              ? projectFilter
              : undefined
          }
          knownProjects={projects.filter((p) => p !== GLOBAL_KEY)}
          onCreated={() => {
            setShowAdd(false);
            void refresh();
          }}
          onCancel={() => setShowAdd(false)}
        />
      )}

      {err && (
        <div
          role="alert"
          style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)", flexWrap: "wrap" }}
        >
          <span style={{ color: "var(--danger)", fontSize: "var(--fs-base)" }}>{err}</span>
          <Button variant="subtle" onClick={() => void refresh()} disabled={loading}>
            {loading ? "Retrying…" : "Retry"}
          </Button>
        </div>
      )}

      {partial && (
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)", fontSize: "var(--fs-sm)", color: "var(--warn)" }}>
          <span>{partial}</span>
          <Button variant="ghost" onClick={() => void refresh()} disabled={loading}>
            {loading ? "…" : "Retry"}
          </Button>
        </div>
      )}

      {truncated.length > 0 && (
        <p style={{ margin: 0, fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
          Showing the {LIMIT} most recent {truncated.join(" and ")}; older ones
          may be hidden — narrow with filters or search to see them.
        </p>
      )}

      {!err && !partial && !loading && filtered.length === 0 && (
        <KnowEmptyState
          projectFilter={projectFilter}
          kindFilter={kindFilter}
          stateFilter={stateFilter}
          query={query}
          onClearProject={() => setProjectFilter("all")}
          onClearState={() => setStateFilter("all")}
          onClearSearch={() => setQuery("")}
          onClearAll={() => {
            setProjectFilter("all");
            setKindFilter("all");
            setStateFilter("all");
            setQuery("");
          }}
        />
      )}

      {groups.map((g) => (
        <ProjectGroup
          key={g.key}
          group={g}
          open={!collapsed.has(g.key)}
          onToggleOpen={() =>
            setCollapsed((prev) => {
              const next = new Set(prev);
              if (next.has(g.key)) next.delete(g.key);
              else next.add(g.key);
              return next;
            })
          }
          focusedKey={focusedKey}
          openProvenanceKey={openProvenance}
          onToggleProvenance={toggleProvenance}
          registerCard={(key, el) => {
            if (el) cardRefs.current.set(key, el);
            else cardRefs.current.delete(key);
          }}
          onArchived={refresh}
          onReview={onReview}
        />
      ))}

      {visible.length > 0 && (
        <p style={{ margin: 0, fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
          <kbd>j</kbd>/<kbd>k</kbd> move · <kbd>enter</kbd> opens the source
          exchange for a memory learned from a transcript. The pipeline
          (Review) is the intake — you judge, never author.
        </p>
      )}
    </div>
  );
}

// ─── one collapsible project group ───────────────────────────────

interface Group {
  key: string;
  projectPath: string | null;
  items: KnowItem[];
  mix: TrustMix;
}

function ProjectGroup({
  group,
  open,
  onToggleOpen,
  focusedKey,
  openProvenanceKey,
  onToggleProvenance,
  registerCard,
  onArchived,
  onReview,
}: {
  group: Group;
  open: boolean;
  onToggleOpen: () => void;
  focusedKey: string | null;
  openProvenanceKey: string | null;
  onToggleProvenance: (key: string) => void;
  registerCard: (key: string, el: HTMLDivElement | null) => void;
  onArchived: () => void;
  onReview: (queue?: "proposed" | "suspect") => void;
}) {
  const name =
    group.projectPath == null ? "Global" : basename(group.projectPath);
  const panelId = `know-group-${group.key}`;

  return (
    <section>
      {/* The collapse toggle and the copy button are siblings, not nested
          (a <button> inside a <button> is invalid). The group header is
          the canonical copy site for the project path within this section
          — the project path is the group's primary identity. */}
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
        <button
          type="button"
          className="pm-focus"
          onClick={onToggleOpen}
          aria-expanded={open}
          aria-controls={panelId}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
            flex: 1,
            minWidth: 0,
            textAlign: "left",
            background: "transparent",
            border: "none",
            padding: "var(--sp-6) 0",
            cursor: "pointer",
            color: "var(--fg)",
            font: "inherit",
          }}
        >
          <Glyph g={open ? NF.chevronD : NF.chevronR} />
          <span
            style={{
              fontWeight: 600,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={group.projectPath ?? "Global memories"}
          >
            {name}
          </span>
          <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
            {group.items.length} item{group.items.length === 1 ? "" : "s"}
          </span>
        </button>
        {group.projectPath && (
          <CopyButton
            text={group.projectPath}
            ariaLabel={`Copy project path ${group.projectPath}`}
          />
        )}
        <div style={{ width: "12rem", maxWidth: "40%" }}>
          <TrustBar mix={group.mix} />
        </div>
      </div>

      {open && (
        <ul
          id={panelId}
          style={{
            listStyle: "none",
            margin: "var(--sp-8) 0 0",
            padding: 0,
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-8)",
          }}
        >
          {group.items.map((it) => {
            const key = `${it.type}:${it.id}`;
            return (
              <KnowItemCard
                key={key}
                item={it}
                focused={focusedKey === key}
                provenanceOpen={openProvenanceKey === key}
                onToggleProvenance={() => onToggleProvenance(key)}
                cardRef={(el) => registerCard(key, el)}
                onArchived={onArchived}
                onReview={onReview}
              />
            );
          })}
        </ul>
      )}
    </section>
  );
}

// ─── filter bar ──────────────────────────────────────────────────

function FilterBar({
  stateFilter,
  kindFilter,
  query,
  projectFilter,
  projects,
  addOpen,
  onState,
  onKind,
  onQuery,
  onProject,
  onToggleAdd,
}: {
  stateFilter: StateFilter;
  kindFilter: KindFilter;
  query: string;
  projectFilter: string;
  projects: string[];
  addOpen: boolean;
  onState: (s: StateFilter) => void;
  onKind: (k: KindFilter) => void;
  onQuery: (q: string) => void;
  onProject: (p: string) => void;
  onToggleAdd: () => void;
}) {
  // A deep-linked project (from the Dashboard's coverage grid) may hold no
  // curated items yet, so it isn't in `projects` — inject it as an option so
  // the control shows the truth ("<name> — uncurated") instead of silently
  // rendering "All projects" while the base stays filtered and empty.
  const injectProject =
    projectFilter !== "all" &&
    projectFilter !== GLOBAL_KEY &&
    !projects.includes(projectFilter);

  return (
    <div style={{ display: "flex", gap: "var(--sp-8)", flexWrap: "wrap", alignItems: "center" }}>
      <Input
        value={query}
        onChange={(e) => onQuery(e.currentTarget.value)}
        placeholder="Filter this base…"
        aria-label="Search knowledge"
        style={{ minWidth: "12rem", flex: "0 1 16rem" }}
      />
      <select
        value={stateFilter}
        onChange={(e) => onState(e.currentTarget.value as StateFilter)}
        aria-label="State filter"
        style={selectStyle()}
      >
        <option value="all">All states</option>
        <option value="proposed">Proposed</option>
        <option value="accepted">Accepted (incl. enforced)</option>
        <option value="enforced">Enforced</option>
        <option value="suspect">Suspect</option>
        <option value="active">Active (decisions)</option>
        <option value="superseded">Superseded (decisions)</option>
      </select>
      <select
        value={kindFilter}
        onChange={(e) => onKind(e.currentTarget.value as KindFilter)}
        aria-label="Kind filter"
        style={selectStyle()}
      >
        <option value="all">All kinds</option>
        <option value="fact">Fact</option>
        <option value="preference">Preference</option>
        <option value="pattern">Pattern</option>
        <option value="constraint">Constraint</option>
        <option value="summary">Summary</option>
        <option value="decision">Decision</option>
        <option value="evidence">Evidence</option>
      </select>
      <select
        value={projectFilter}
        onChange={(e) => onProject(e.currentTarget.value)}
        aria-label="Project filter"
        style={{ ...selectStyle(), maxWidth: "16rem" }}
      >
        <option value="all">All projects</option>
        {injectProject && (
          <option value={projectFilter}>{basename(projectFilter)} — uncurated</option>
        )}
        {projects.map((p) => (
          <option key={p} value={p}>
            {p === GLOBAL_KEY ? "Global" : basename(p)}
          </option>
        ))}
      </select>
      <div style={{ flex: 1 }} />
      {/* Secondary intake — the pipeline (Review) is the primary way
          knowledge enters the base, so "Add" stays a ghost affordance. */}
      <Button variant="ghost" glyph={NF.plus} onClick={onToggleAdd}>
        {addOpen ? "Cancel" : "Add"}
      </Button>
    </div>
  );
}

// ─── empty state ─────────────────────────────────────────────────
//
// Never a dead-end: the empty state names the filter that emptied the view
// and offers the exact way out. The most common trigger is a Dashboard
// deep-link into a busy-but-uncurated project — so that case leads with the
// project's own harvest command, not a generic "accept in Review".

function KnowEmptyState({
  projectFilter,
  kindFilter,
  stateFilter,
  query,
  onClearProject,
  onClearState,
  onClearSearch,
  onClearAll,
}: {
  projectFilter: string;
  kindFilter: KindFilter;
  stateFilter: StateFilter;
  query: string;
  onClearProject: () => void;
  onClearState: () => void;
  onClearSearch: () => void;
  onClearAll: () => void;
}) {
  const evidenceStateClash = kindFilter === "evidence" && stateFilter !== "all";
  const searching = query.trim().length > 0;
  const otherFilterActive = kindFilter !== "all" || stateFilter !== "all";
  // "Nothing curated in <project>" is only the right message when the project
  // filter is the SOLE reason the view is empty. If a search or a kind/state
  // filter is also active, that is the real cause — and "Clear project
  // filter" wouldn't fix it — so defer to those branches instead.
  const projectScoped =
    projectFilter !== "all" &&
    projectFilter !== GLOBAL_KEY &&
    !searching &&
    !otherFilterActive;

  let title: React.ReactNode;
  let detail: React.ReactNode = null;
  let action: React.ReactNode = null;

  if (projectScoped) {
    const cmd = `claudepot lesson harvest --project ${shellQuote(projectFilter)}`;
    title = (
      <>
        Nothing curated in <strong>{basename(projectFilter)}</strong> yet.
      </>
    );
    detail = (
      <>
        Harvest this project’s sessions, then judge what surfaces in Review:
        <br />
        <code style={{ wordBreak: "break-all" }}>{cmd}</code>{" "}
        <CopyButton text={cmd} ariaLabel="Copy harvest command" />
      </>
    );
    action = (
      <Button variant="ghost" onClick={onClearProject}>
        Clear project filter
      </Button>
    );
  } else if (evidenceStateClash) {
    title = <>Evidence has no lifecycle state.</>;
    detail = <>Clear the State filter to see evidence records.</>;
    action = (
      <Button variant="ghost" onClick={onClearState}>
        Clear state filter
      </Button>
    );
  } else if (searching) {
    title = <>No curated records match “{query.trim()}”.</>;
    action = (
      <Button variant="ghost" onClick={onClearSearch}>
        Clear search
      </Button>
    );
  } else {
    title = <>Nothing curated matches these filters.</>;
    detail = (
      <>
        The pipeline is the intake — accept lessons in Review, or harvest more
        with <code>claudepot lesson harvest</code>.
      </>
    );
    action = otherFilterActive ? (
      <Button variant="ghost" onClick={onClearAll}>
        Clear filters
      </Button>
    ) : null;
  }

  return (
    <div
      style={{
        border: "var(--sp-px) dashed var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-24)",
        textAlign: "center",
        color: "var(--fg-muted)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        alignItems: "center",
      }}
    >
      <p style={{ margin: 0 }}>{title}</p>
      {detail && <p style={{ margin: 0, fontSize: "var(--fs-sm)" }}>{detail}</p>}
      {action}
    </div>
  );
}

// ─── filtering + grouping ────────────────────────────────────────

function matchesProject(it: KnowItem, filter: string): boolean {
  if (filter === "all") return true;
  return (it.projectPath ?? GLOBAL_KEY) === filter;
}

function matchesKind(it: KnowItem, filter: KindFilter): boolean {
  if (filter === "all") return true;
  if (filter === "decision") return it.type === "decision";
  if (filter === "evidence") return it.type === "evidence";
  return it.type === "memory" && it.row.kind === filter;
}

function matchesState(it: KnowItem, filter: StateFilter): boolean {
  if (filter === "all") return true;
  switch (it.type) {
    case "memory": {
      const label = memoryStateBadge(it.row).label;
      // "Accepted" is the superset: an enforced lesson is still accepted
      // (accepted + compiled). Filtering "Accepted" must not hide exactly
      // the strongest, guard-backed items. "Enforced" is the narrower view.
      if (filter === "accepted") return label === "accepted" || label === "enforced";
      return label === filter;
    }
    case "decision":
      return decisionStateBadge(it.row).label === filter;
    case "evidence":
      // Evidence has no lifecycle state; a specific-state filter is asking
      // for lifecycle items, so it drops out.
      return false;
  }
}

/** Free-text match over the record's own words (claim / decision / summary,
 *  plus directive, rationale, topic, kind). Searches the curated base — not
 *  raw transcripts, which is Recall's job. */
function matchesText(it: KnowItem, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay =
    it.type === "memory"
      ? `${it.row.content} ${it.row.directive ?? ""} ${it.row.kind}`
      : it.type === "decision"
        ? `${it.row.decision} ${it.row.rationale ?? ""} ${it.row.topic ?? ""}`
        : `${it.row.summary} ${it.row.verification} ${it.row.topic ?? ""}`;
  return hay.toLowerCase().includes(q);
}

export function groupByProject(items: KnowItem[]): Group[] {
  const map = new Map<string, KnowItem[]>();
  for (const it of items) {
    const key = it.projectPath ?? GLOBAL_KEY;
    const arr = map.get(key) ?? [];
    arr.push(it);
    map.set(key, arr);
  }
  const groups: Group[] = [];
  for (const [key, arr] of map) {
    arr.sort((a, b) => b.createdAtMs - a.createdAtMs);
    groups.push({
      key,
      projectPath: key === GLOBAL_KEY ? null : key,
      items: arr,
      mix: mixFromItems(arr),
    });
  }
  // Groups needing attention (most suspect) first, then by size.
  groups.sort((a, b) => {
    if (b.mix.suspect !== a.mix.suspect) return b.mix.suspect - a.mix.suspect;
    if (b.items.length !== a.items.length) return b.items.length - a.items.length;
    return a.key.localeCompare(b.key);
  });
  return groups;
}

/** The trust mix for a project header, computed from the memory items in
 *  the group (decisions/evidence don't carry a trust state). */
function mixFromItems(items: KnowItem[]): TrustMix {
  const mix: TrustMix = { enforced: 0, documented: 0, suspect: 0, proposed: 0 };
  for (const it of items) {
    if (it.type !== "memory") continue;
    const row = it.row;
    if (row.review_state === "proposed") mix.proposed += 1;
    else if (row.review_state === "suspect") mix.suspect += 1;
    else if (row.review_state === "accepted")
      isEnforced(row) ? (mix.enforced += 1) : (mix.documented += 1);
  }
  return mix;
}

function selectStyle(): React.CSSProperties {
  return {
    padding: "0 var(--sp-8)",
    height: "var(--input-height)",
    background: "var(--bg-raised)",
    color: "var(--fg)",
    border: "var(--sp-px) solid var(--line)",
    borderRadius: "var(--r-2)",
    font: "inherit",
  };
}

/** Single-quote a path for safe shell interpolation — a project path can
 *  contain spaces, so the copied harvest command must quote it or the shell
 *  would split it into the wrong args. */
function shellQuote(s: string): string {
  return `'${s.replace(/'/g, "'\\''")}'`;
}
