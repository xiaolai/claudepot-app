import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { copyToClipboard } from "../lib/copyToClipboard";
import { useSessionsData } from "../hooks/useSessionsData";
import { useTrashCount } from "../hooks/useTrashCount";
import {
  readSessionsFilter,
  writeSessionsFilter,
} from "./sessions/sessionsFilterStore";
import { ContextMenu } from "../components/ContextMenu";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { Toast } from "../components/primitives/Toast";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useSessionSearch } from "../hooks/useSessionSearch";
import { useCompactHeader, useSplitView } from "../hooks/useWindowWidth";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import type { SessionRow } from "../types";
import { MoveSessionModal } from "./projects/MoveSessionModal";
import { CleanupPane } from "./sessions/CleanupPane";
import { SessionsTrendsPane } from "./sessions/components/SessionsTrendsPane";
import { useSessionLive } from "../hooks/useSessionLive";
import { SectionTab } from "./sessions/components/SectionTab";
import { SessionsTabPanel } from "./sessions/components/SessionsTabPanel";
import { buildSessionContextMenuItems } from "./sessions/sessionsContextMenu";
import { filterSessionsByRepo } from "./sessions/RepoFilterStrip";
import {
  buildSessionSearchHaystack,
  matchesQuery,
} from "./sessions/sessionSearchIndex";
import {
  countSessionStatus,
  type SessionFilter,
} from "./sessions/SessionsTable";
import { TrashDrawer } from "./sessions/TrashDrawer";

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
  // Seed local state from the module-scope filter store so a user
  // who typed a query, picked a repo, or drilled into a transcript
  // returns to the same view after a cross-section hop. On mount the
  // snapshot is copied in; on change we write back. The store never
  // pushes updates — consumers read once and own their copy after.
  const initialFilters = useRef(readSessionsFilter());
  const [activeRepo, setActiveRepoRaw] = useState<string | null>(
    initialFilters.current.activeRepo,
  );
  /** The file_path of the selected row — globally unique on disk.
   * We key selection by path (not session_id) because CC can end up
   * with two .jsonl files that share a session_id (e.g. an interrupted
   * adopt_orphan left the source file behind). */
  const [selectedPath, setSelectedPathRaw] = useState<string | null>(
    initialFilters.current.selectedPath,
  );
  const [filter, setFilterRaw] = useState<SessionFilter>(
    initialFilters.current.filter,
  );
  const [query, setQueryRaw] = useState(initialFilters.current.query);
  const [tab, setTabRaw] = useState<"sessions" | "trends" | "cleanup">(
    initialFilters.current.tab,
  );
  const [liveFilter, setLiveFilterRaw] = useState<boolean>(
    initialFilters.current.liveFilter,
  );
  // Wrap each setter so every mutation funnels through the module
  // store. Using a typed helper keeps the call sites unchanged at
  // the hook-consumption level.
  const setActiveRepo = useCallback<typeof setActiveRepoRaw>((next) => {
    setActiveRepoRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ activeRepo: resolved });
      return resolved;
    });
  }, []);
  const setSelectedPath = useCallback<typeof setSelectedPathRaw>((next) => {
    setSelectedPathRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ selectedPath: resolved });
      return resolved;
    });
  }, []);
  const setFilter = useCallback<typeof setFilterRaw>((next) => {
    setFilterRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ filter: resolved });
      return resolved;
    });
  }, []);
  const setQuery = useCallback<typeof setQueryRaw>((next) => {
    setQueryRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ query: resolved });
      return resolved;
    });
  }, []);
  const setTab = useCallback<typeof setTabRaw>((next) => {
    setTabRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ tab: resolved });
      return resolved;
    });
  }, []);
  const setLiveFilter = useCallback<typeof setLiveFilterRaw>((next) => {
    setLiveFilterRaw((prev) => {
      const resolved = typeof next === "function" ? next(prev) : next;
      writeSessionsFilter({ liveFilter: resolved });
      return resolved;
    });
  }, []);
  const [detailRefreshSignal, setDetailRefreshSignal] = useState(0);
  const [toast, setToast] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    session: SessionRow;
  } | null>(null);
  const [moveSession, setMoveSession] = useState<SessionRow | null>(null);

  // Data lifecycle (refresh + Promise.allSettled + cancellation token)
  // lives in the hook. The hook surfaces secondary-fetch failures via
  // the `onSecondaryError` callback, which we bridge to the toast.
  const {
    sessions,
    projects,
    repoGroups,
    loading,
    error,
    servedFromCache,
    fetchStartedAt,
    refresh,
  } = useSessionsData({ onSecondaryError: setToast });

  // Dot on the Cleanup tab when trash is non-empty. Render-if-nonzero.
  const { count: trashCount, refresh: refreshTrash } = useTrashCount();

  // Live-runtime snapshot used by the Live filter chip. `useSessionLive`
  // is a module-scope store so this is cheap even though the Sessions
  // section is lazy.
  const liveSummaries = useSessionLive();
  const liveSessionIds = useMemo(
    () => new Set(liveSummaries.map((s) => s.session_id)),
    [liveSummaries],
  );

  // Tick once a second while a fetch is in flight so the "Updating…
  // Ns" elapsed-time label ages without re-rendering the whole table
  // between ticks. Only runs while `fetchStartedAt` is non-null — no
  // idle polling cost.
  const [, setElapsedTick] = useState(0);
  useEffect(() => {
    if (fetchStartedAt === null) return;
    const id = window.setInterval(() => setElapsedTick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [fetchStartedAt]);
  const elapsedLabel = useMemo(() => {
    if (fetchStartedAt === null) return null;
    const secs = Math.max(0, Math.floor((Date.now() - fetchStartedAt) / 1000));
    if (secs < 1) return "Updating…";
    return `Updating… ${secs}s`;
  }, [fetchStartedAt]);

  // Selection / repo-filter pruning lives here (not in the hook)
  // because both are owned by this component. Run as effects on the
  // dataset, so a stale selection or repo id from a prior dataset
  // self-clears once `sessions` / `repoGroups` lands.
  useEffect(() => {
    setSelectedPath((prev) =>
      prev && sessions.some((s) => s.file_path === prev) ? prev : null,
    );
  }, [sessions]);
  useEffect(() => {
    setActiveRepo((prev) =>
      prev &&
      repoGroups &&
      repoGroups.some((g) => (g.repo_root ?? g.label) === prev)
        ? prev
        : null,
    );
  }, [repoGroups]);

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
    let rows = scoped;
    if (q) {
      rows = rows.filter(
        (s) =>
          matchesQuery(s, haystack, q) ||
          // Deep content hit from the backend search.
          deepHitPaths.has(s.file_path),
      );
    }
    if (liveFilter) {
      rows = rows.filter((s) => liveSessionIds.has(s.session_id));
    }
    return rows;
  }, [scoped, deferredQuery, deepHitPaths, haystack, liveFilter, liveSessionIds]);

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
          <>
            {elapsedLabel && (
              <span
                className="mono-cap"
                aria-live="polite"
                style={{
                  fontSize: "var(--fs-2xs)",
                  color: servedFromCache
                    ? "var(--fg-faint)"
                    : "var(--fg-ghost)",
                  letterSpacing: "0.03em",
                }}
                title={
                  servedFromCache
                    ? "Showing cached rows while we re-scan transcripts"
                    : undefined
                }
              >
                {elapsedLabel}
              </span>
            )}
            {compact ? (
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
                title="Refresh session list (⌘R)"
              >
                Refresh sessions
              </Button>
            )}
          </>
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
          id="sessions-tab-trends"
          panelId="sessions-tab-panel-trends"
          label="Trends"
          active={tab === "trends"}
          onSelect={() => setTab("trends")}
        />
        <SectionTab
          id="sessions-tab-cleanup"
          panelId="sessions-tab-panel-cleanup"
          label="Cleanup"
          active={tab === "cleanup"}
          onSelect={() => setTab("cleanup")}
          indicator={trashCount > 0 ? true : undefined}
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
            <CleanupPane
              onTrashChanged={() => {
                refresh();
                refreshTrash();
              }}
              setToast={setToast}
            />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <TrashDrawer onChange={refresh} />
          </div>
        </div>
      )}

      {tab === "sessions" && (
        <SessionsTabPanel
          showTable={showTable}
          showDetail={showDetail}
          splitView={splitView}
          repoGroups={repoGroups}
          activeRepo={activeRepo}
          setActiveRepo={setActiveRepo}
          query={query}
          setQuery={setQuery}
          filter={filter}
          setFilter={setFilter}
          counts={counts}
          loading={loading}
          error={error}
          sessions={sessions}
          filteredByQuery={filteredByQuery}
          searchSnippets={searchSnippets}
          selectedPath={selectedPath}
          setSelectedPath={setSelectedPath}
          projects={projects}
          detailRefreshSignal={detailRefreshSignal}
          setDetailRefreshSignal={setDetailRefreshSignal}
          onContextMenu={handleContextMenu}
          onRefresh={refresh}
          setToast={setToast}
          liveFilter={liveFilter}
          setLiveFilter={setLiveFilter}
          liveCount={liveSummaries.length}
        />
      )}
      {tab === "trends" && (
        <div
          id="sessions-tab-panel-trends"
          role="tabpanel"
          aria-labelledby="sessions-tab-trends"
          style={{
            flex: 1,
            minHeight: 0,
            overflow: "auto",
          }}
        >
          <SessionsTrendsPane />
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

      {ctxMenu && (
        <ContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          items={buildSessionContextMenuItems({
            session: ctxMenu.session,
            setToast,
            setSelectedPath,
            setMoveSession,
            copyToClipboard: (text, label) =>
              copyToClipboard(text, label, setToast),
          })}
          onClose={closeCtxMenu}
        />
      )}
    </>
  );
}

