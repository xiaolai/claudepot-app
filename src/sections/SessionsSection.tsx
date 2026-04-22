import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { api } from "../api";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { Button } from "../components/primitives/Button";
import { FilterChip } from "../components/primitives/FilterChip";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
import { Toast } from "../components/primitives/Toast";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useSessionSearch } from "../hooks/useSessionSearch";
import { useCompactHeader, useSplitView } from "../hooks/useWindowWidth";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import type { ProjectInfo, RepositoryGroup, SessionRow } from "../types";
import { MoveSessionModal } from "./projects/MoveSessionModal";
import { CleanupPane } from "./sessions/CleanupPane";
import {
  RepoFilterStrip,
  filterSessionsByRepo,
} from "./sessions/RepoFilterStrip";
import { SessionDetail } from "./sessions/SessionDetail";
import {
  buildSessionSearchHaystack,
  matchesQuery,
} from "./sessions/sessionSearchIndex";
import {
  SessionsTable,
  countSessionStatus,
  type SessionFilter,
} from "./sessions/SessionsTable";
import { TrashDrawer } from "./sessions/TrashDrawer";

/**
 * Toggleable chips: each flips the filter between "all" and its own
 * value. Two chips active at once is not a supported state — picking
 * one deselects the other (mutual exclusion preserves the existing
 * `SessionFilter` enum shape).
 */
const FILTER_CHIPS: { id: Exclude<SessionFilter, "all">; label: string }[] = [
  { id: "errors", label: "Errors" },
  { id: "sidechain", label: "Agents" },
];

/**
 * Sessions tab — a flat, cross-project index of every CC session on
 * disk. Mirrors the Projects section's layout: table on the left,
 * detail (transcript) on the right, with a single-pane master/detail
 * flow below `useSplitView()`'s minimum width.
 *
 * Data sources:
 *  - `api.sessionListAll()` → full metadata row per `.jsonl` (backed
 *    by a parallel rayon scan in Rust). Powers the table.
 *  - `api.sessionRead(sessionId)` → full transcript events for the
 *    selected row. Lazy per-selection, not prefetched.
 *  - `api.projectList()` → feeds the MoveSessionModal target picker.
 *
 * The `refresh()` handler re-pulls sessions AND projects in parallel
 * so the Move modal's target list stays fresh after a move.
 */
export interface SessionsSectionProps {
  /**
   * Path the caller wants pre-selected (e.g. from a cross-session
   * command-palette hit). Consumed exactly once on mount; use a
   * parent-owned key/state rotation to re-prime.
   */
  initialSelectedPath?: string | null;
  onInitialSelectedPathConsumed?: () => void;
}

export function SessionsSection(props: SessionsSectionProps = {}) {
  const { initialSelectedPath = null, onInitialSelectedPathConsumed } = props;
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [repoGroups, setRepoGroups] = useState<RepositoryGroup[] | null>(null);
  const [activeRepo, setActiveRepo] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** The file_path of the selected row — globally unique on disk.
   * We key selection by path (not session_id) because CC can end up
   * with two .jsonl files that share a session_id (e.g. an interrupted
   * adopt_orphan left the source file behind). */
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [filter, setFilter] = useState<SessionFilter>("all");
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"sessions" | "cleanup">("sessions");
  const [detailRefreshSignal, setDetailRefreshSignal] = useState(0);
  const [toast, setToast] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    session: SessionRow;
  } | null>(null);
  const [moveSession, setMoveSession] = useState<SessionRow | null>(null);

  const tokenRef = useRef(0);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(() => {
    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    // Use allSettled so a failure in projectList or sessionWorktreeGroups
    // doesn't blank the whole Sessions section. The session list is the
    // only mandatory dependency — if it loads, the table renders.
    // Secondary failures degrade their corresponding UI surface
    // (RepoFilterStrip vanishes, MoveSessionModal target list is empty)
    // and surface as a single inline toast.
    Promise.allSettled([
      api.sessionListAll(),
      api.projectList(),
      api.sessionWorktreeGroups(),
    ])
      .then(([ssRes, psRes, groupsRes]) => {
        if (!mountedRef.current || myToken !== tokenRef.current) return;

        // Mandatory: the session list. If this rejects, we can't
        // render the table and must surface the error inline.
        if (ssRes.status === "rejected") {
          setError(String(ssRes.reason));
          setLoading(false);
          return;
        }
        const ss = ssRes.value;
        setSessions(ss);

        // Secondary: projects (target list for Move modal). On
        // rejection we fall back to an empty list and toast once.
        if (psRes.status === "fulfilled") {
          setProjects(psRes.value);
        } else {
          setProjects([]);
          setToast(`Couldn't load projects: ${String(psRes.reason)}`);
          if (import.meta.env.DEV) {
            // eslint-disable-next-line no-console
            console.warn("[SessionsSection] projectList failed", psRes.reason);
          }
        }

        // Tertiary: worktree groups (powers RepoFilterStrip). Already
        // designed to be optional — rejection just hides the strip.
        if (groupsRes.status === "fulfilled") {
          setRepoGroups(groupsRes.value);
        } else {
          setRepoGroups(null);
          if (import.meta.env.DEV) {
            // eslint-disable-next-line no-console
            console.warn(
              "[SessionsSection] sessionWorktreeGroups failed; " +
                "RepoFilterStrip will not render",
              groupsRes.reason,
            );
          }
        }

        setLoading(false);
        // Drop the selection if it no longer exists.
        setSelectedPath((prev) =>
          prev && ss.some((s) => s.file_path === prev) ? prev : null,
        );
        // Drop the active repo id if the new groups don't contain it.
        // Id is `repo_root` for git-tracked repos, `label` for no-repo.
        const groups =
          groupsRes.status === "fulfilled" ? groupsRes.value : null;
        setActiveRepo((prev) =>
          prev &&
          groups &&
          groups.some((g) => (g.repo_root ?? g.label) === prev)
            ? prev
            : null,
        );
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Consume the deep-link path from `initialSelectedPath` exactly once
  // per mount. Runs when the prop flips from falsy to a value — the
  // parent state rotation is the trigger, not a timer.
  useEffect(() => {
    if (initialSelectedPath) {
      setSelectedPath(initialSelectedPath);
      setActiveRepo(null); // clear repo filter so the selection is visible
      onInitialSelectedPathConsumed?.();
    }
  }, [initialSelectedPath, onInitialSelectedPathConsumed]);

  useGlobalShortcuts({ onRefresh: refresh });

  const counts = useMemo(() => countSessionStatus(sessions), [sessions]);

  // `useDeferredValue` decouples the filter + deep-search pipeline from
  // keystrokes. The input stays instant because its controlled value is
  // the raw `query`; the filter recomputes at a lower priority, which
  // React interrupts when a newer keystroke arrives. The result: no
  // "semi-frozen" typing even when `sessions` is several thousand rows.
  const deferredQuery = useDeferredValue(query);

  // Pre-lowercased haystack for the filter. Rebuilt only when
  // `sessions` changes, not on every keystroke — so each keystroke
  // walks the list in O(n) substring-checks against cached strings
  // instead of re-lowercasing 5–6 fields per row.
  const haystack = useMemo(
    () => buildSessionSearchHaystack(sessions),
    [sessions],
  );

  // Table-level name filter: matches on project basename, project path,
  // first prompt, session id prefix, model, or git branch. Cheap substring.
  // Stacks on top of the repo filter so "lixiaolai.com / feature/x" is
  // trivially reachable.
  const scoped = useMemo(
    () => filterSessionsByRepo(sessions, repoGroups, activeRepo),
    [sessions, repoGroups, activeRepo],
  );

  // Deep content search (useSessionSearch): scans transcript bodies so
  // a query like "deadlock" surfaces sessions whose metadata doesn't
  // mention the word. Debounced + 2-char min inside the hook. Driven
  // by the deferred value so the heavy client-side filter re-render
  // doesn't retrigger the hook's own debounce timer on every frame
  // under load; the hook's `requestSeqRef` guard still handles
  // out-of-order IPC responses.
  const { hits: deepHits, error: deepSearchError } = useSessionSearch(
    deferredQuery,
    50,
  );

  // Surface deep-search IPC failures. The hook reports one error per
  // query that rejects; bridging it to the toast surface keeps
  // "silent empty results" from looking like a legitimate zero-hit
  // search. We de-dupe on the string so a stable error from a stuck
  // query doesn't re-toast on every re-render.
  const lastReportedDeepErr = useRef<string | null>(null);
  useEffect(() => {
    if (deepSearchError && deepSearchError !== lastReportedDeepErr.current) {
      lastReportedDeepErr.current = deepSearchError;
      setToast(`Couldn't search sessions: ${deepSearchError}`);
    } else if (!deepSearchError) {
      lastReportedDeepErr.current = null;
    }
  }, [deepSearchError]);
  const deepHitPaths = useMemo(
    () => new Set(deepHits.map((h) => h.file_path)),
    [deepHits],
  );
  /** `file_path → snippet` map used by the table to show match context.
   * Snippets are already redacted by the backend (see
   * session_search::make_hit → redact_secrets), so sk-ant- substrings
   * never reach the DOM. */
  const searchSnippets = useMemo(() => {
    const m = new Map<string, string>();
    for (const h of deepHits) m.set(h.file_path, h.snippet);
    return m;
  }, [deepHits]);
  const filteredByQuery = useMemo(() => {
    const q = deferredQuery.trim().toLowerCase();
    if (!q) return scoped;
    return scoped.filter(
      (s) =>
        matchesQuery(s, haystack, q) ||
        // Deep content hit from the backend search.
        deepHitPaths.has(s.file_path),
    );
  }, [scoped, deferredQuery, deepHitPaths, haystack]);

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, s: SessionRow) => {
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY, session: s });
    },
    [],
  );
  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  const compact = useCompactHeader();
  const splitView = useSplitView();
  const showDetail = selectedPath !== null;
  const showTable = splitView || selectedPath === null;

  // Count the rows the table will actually render. Includes every
  // narrowing the user applied: repo (scoped), query (deferred +
  // deep-hit), AND the status chip (`filter`). Without this last
  // step the subtitle would announce a global count while the table
  // is scoped to one status filter — a UI lie. Counting here mirrors
  // the same predicate the table uses; the duplication is intentional
  // because `SessionsTable` owns the sort, not the filter, and we
  // need the count before the sort runs.
  const visibleCount = useMemo(() => {
    if (filter === "all") return filteredByQuery.length;
    return filteredByQuery.reduce((n, s) => {
      if (filter === "errors" && s.has_error) return n + 1;
      if (filter === "sidechain" && s.is_sidechain) return n + 1;
      return n;
    }, 0);
  }, [filteredByQuery, filter]);

  const subtitle = (() => {
    if (error && sessions.length === 0) return "Couldn't load sessions.";
    const total = sessions.length;
    if (total === 0) return "No sessions yet. Run `claude` to start one.";
    // Narrowed if any of: query, repo, or status filter is active.
    // Use deferredQuery (not raw query) for the same reason the table
    // does — a one-tick discrepancy between subtitle and visible rows
    // would lie about the UI state.
    const narrowed = visibleCount !== total;
    if (narrowed) {
      return `${visibleCount} of ${total} session${
        total === 1 ? "" : "s"
      } shown`;
    }
    const pieces: string[] = [`${total} session${total === 1 ? "" : "s"}`];
    if (counts.errors > 0) pieces.push(`${counts.errors} with errors`);
    if (counts.sidechain > 0) pieces.push(`${counts.sidechain} agent`);
    return pieces.join(" · ");
  })();

  return (
    <>
      <ScreenHeader
        title="Sessions"
        subtitle={subtitle}
        actions={
          compact ? (
            <IconButton
              glyph={NF.refresh}
              onClick={refresh}
              title="Refresh (⌘R)"
              aria-label="Refresh sessions"
            />
          ) : (
            <Button
              variant="ghost"
              glyph={NF.refresh}
              glyphColor="var(--fg-muted)"
              onClick={refresh}
              title="Refresh (⌘R)"
            >
              Refresh
            </Button>
          )
        }
      />

      <div
        role="tablist"
        aria-label="Sessions view"
        style={{
          display: "flex",
          gap: "var(--sp-6)",
          padding: "var(--sp-8) var(--sp-32)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          background: "var(--bg)",
        }}
      >
        <SectionTab
          id="sessions-tab-sessions"
          panelId="sessions-tab-panel-sessions"
          label="Sessions"
          active={tab === "sessions"}
          onSelect={() => setTab("sessions")}
        />
        <SectionTab
          id="sessions-tab-cleanup"
          panelId="sessions-tab-panel-cleanup"
          label="Cleanup"
          active={tab === "cleanup"}
          onSelect={() => setTab("cleanup")}
        />
      </div>

      {tab === "cleanup" && (
        <div
          id="sessions-tab-panel-cleanup"
          role="tabpanel"
          aria-labelledby="sessions-tab-cleanup"
          style={{
            display: "flex",
            flex: 1,
            minHeight: 0,
            overflow: "auto",
          }}
        >
          <div style={{ flex: 2, minWidth: 0, borderRight: "var(--bw-hair) solid var(--line)" }}>
            <CleanupPane onTrashChanged={refresh} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <TrashDrawer onChange={refresh} />
          </div>
        </div>
      )}

      {tab === "sessions" && (
        // Single tabpanel wrapping all sessions-tab content. We use
        // `display: contents` so the existing column-of-flex-children
        // layout (RepoFilterStrip + filter row + error pane / table+
        // detail) survives the wrapping div without any flex-sizing
        // adjustment. Modern AT (NVDA, JAWS, VoiceOver) treats the
        // ARIA role as authoritative regardless of the layout box.
        <div
          id="sessions-tab-panel-sessions"
          role="tabpanel"
          aria-labelledby="sessions-tab-sessions"
          style={{ display: "contents" }}
        >
      {showTable && (
        <RepoFilterStrip
          groups={repoGroups}
          activeRepo={activeRepo}
          onChange={setActiveRepo}
        />
      )}

      {showTable && (
        <div
          style={{
            padding: "var(--sp-14) var(--sp-32)",
            borderBottom: "var(--bw-hair) solid var(--line)",
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--sp-12)",
            alignItems: "center",
            background: "var(--bg)",
            flexShrink: 0,
          }}
        >
          <Input
            glyph={NF.search}
            placeholder="Search project, prompt, content, model, or id"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query.length > 0) {
                e.preventDefault();
                e.stopPropagation();
                setQuery("");
              }
            }}
            style={{
              flex: "1 1 var(--filter-input-width)",
              minWidth: "var(--filter-input-min)",
              maxWidth: "var(--filter-input-width)",
            }}
            aria-label="Search sessions"
          />

          <div
            role="group"
            aria-label="Session filters"
            style={{
              display: "flex",
              gap: "var(--sp-6)",
            }}
          >
            {FILTER_CHIPS.map((opt) => {
              const active = filter === opt.id;
              const count = counts[opt.id];
              return (
                <FilterChip
                  key={opt.id}
                  active={active}
                  count={count}
                  onToggle={() => setFilter(active ? "all" : opt.id)}
                >
                  {opt.label}
                </FilterChip>
              );
            })}
          </div>

          <div style={{ flex: 1 }} />
          {loading && sessions.length > 0 && (
            <span
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
                display: "flex",
                alignItems: "center",
                gap: "var(--sp-6)",
              }}
            >
              <Glyph g={NF.refresh} />
              Refreshing…
            </span>
          )}
        </div>
      )}

      {error && sessions.length === 0 ? (
        <div
          style={{
            flex: 1,
            minHeight: 0,
            overflow: "auto",
            padding: "var(--sp-48)",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--sp-12)",
          }}
        >
          <h2 style={{ fontSize: "var(--fs-lg)", margin: 0 }}>
            Couldn't load sessions
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-xs)",
              margin: 0,
            }}
          >
            {error}
          </p>
          <Button variant="solid" onClick={refresh}>
            Retry
          </Button>
        </div>
      ) : (
        <div style={{ display: "flex", minHeight: 0, flex: 1 }}>
          {showTable && (
            // SessionsTable owns its own scroll container so the row
            // virtualizer has a stable parent to observe. This wrapper
            // only contributes flex sizing; `minHeight: 0` is what
            // lets the inner scroller actually shrink below content.
            <div
              style={{
                flex: 1,
                minWidth: 0,
                minHeight: 0,
                display: "flex",
                flexDirection: "column",
              }}
            >
              <SessionsTable
                sessions={filteredByQuery}
                filter={filter}
                selectedId={selectedPath}
                onSelect={setSelectedPath}
                onContextMenu={handleContextMenu}
                searchSnippets={
                  searchSnippets.size > 0 ? searchSnippets : undefined
                }
              />
            </div>
          )}

          {showDetail && selectedPath && (
            <aside
              style={{
                width: splitView ? "var(--project-detail-width)" : "100%",
                flex: splitView ? "0 0 auto" : "1 1 auto",
                flexShrink: splitView ? 0 : 1,
                borderLeft: splitView
                  ? "var(--bw-hair) solid var(--line)"
                  : "none",
                background: splitView ? "var(--bg-sunken)" : "var(--bg)",
                overflow: "hidden",
                minWidth: 0,
                display: "flex",
                flexDirection: "column",
              }}
            >
              <SessionDetail
                key={selectedPath}
                filePath={selectedPath}
                projects={projects}
                refreshSignal={detailRefreshSignal}
                onMoved={() => {
                  setDetailRefreshSignal((n) => n + 1);
                  refresh();
                }}
                onError={(msg) => setToast(msg)}
                onBack={splitView ? undefined : () => setSelectedPath(null)}
              />
            </aside>
          )}
        </div>
      )}
        </div>
      )}

      {moveSession && (
        <MoveSessionModal
          sessionId={moveSession.session_id}
          fromCwd={moveSession.project_path}
          projects={projects}
          onClose={() => setMoveSession(null)}
          onCompleted={() => {
            setMoveSession(null);
            setDetailRefreshSignal((n) => n + 1);
            refresh();
          }}
        />
      )}

      <Toast message={toast} onDismiss={() => setToast(null)} />

      {ctxMenu &&
        (() => {
          const s = ctxMenu.session;
          const items: ContextMenuItem[] = [
            {
              label: "Open in Finder",
              onClick: () => {
                api.revealInFinder(s.file_path).catch((e) => {
                  setToast(`Couldn't reveal: ${e}`);
                });
              },
            },
            {
              label: "Open in detail",
              onClick: () => setSelectedPath(s.file_path),
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Move to project…",
              onClick: () => {
                if (!s.project_from_transcript) {
                  setToast(
                    "Can't move: this session has no cwd recorded in the transcript.",
                  );
                  return;
                }
                setMoveSession(s);
              },
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Copy session id",
              onClick: () => {
                copyToClipboard(s.session_id, "session id", setToast);
              },
            },
            {
              label: "Copy project path",
              onClick: () => {
                copyToClipboard(s.project_path, "project path", setToast);
              },
            },
          ];
          return (
            <ContextMenu
              x={ctxMenu.x}
              y={ctxMenu.y}
              items={items}
              onClose={closeCtxMenu}
            />
          );
        })()}
    </>
  );
}

/**
 * Copy text to the clipboard, surfacing both success and failure as
 * toasts. The native `navigator.clipboard.writeText` rejects in
 * narrow but real cases (no document focus, HTTPS-only contexts in
 * some browsers, Tauri webview policies) — silently toasting success
 * lies to the user. We `void`-type the promise to satisfy the no-
 * floating-promises lint while keeping the call site synchronous.
 */
function copyToClipboard(
  text: string,
  label: string,
  setToast: (msg: string) => void,
): void {
  void navigator.clipboard.writeText(text).then(
    () => setToast(`Copied ${label}.`),
    (e) => setToast(`Couldn't copy ${label}: ${e instanceof Error ? e.message : String(e)}`),
  );
}

/**
 * Proper `role="tab"` button for the Sessions/Cleanup tab strip. The
 * pre-existing FilterChip uses `role="switch"`, which conflicts with
 * `role="tablist"` parents — assistive tech announces them as toggle
 * switches instead of tabs. This thin button mirrors FilterChip's
 * paper-mono styling but emits the correct ARIA contract: `role=tab`,
 * `aria-selected`, `aria-controls` linking to the tabpanel.
 */
function SectionTab({
  id,
  panelId,
  label,
  active,
  onSelect,
}: {
  id: string;
  panelId: string;
  label: string;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      id={id}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={panelId}
      tabIndex={active ? 0 : -1}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        height: "var(--sp-24)",
        padding: "0 var(--sp-10)",
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        background: active ? "var(--accent-soft)" : "var(--bg-sunken)",
        border: `var(--bw-hair) solid ${active ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-1)",
        cursor: "pointer",
        whiteSpace: "nowrap",
        outlineOffset: 2,
      }}
    >
      {label}
    </button>
  );
}
