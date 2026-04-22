import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { Button } from "../components/primitives/Button";
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
import {
  RepoFilterStrip,
  filterSessionsByRepo,
} from "./sessions/RepoFilterStrip";
import { SessionDetail } from "./sessions/SessionDetail";
import {
  SessionsTable,
  countSessionStatus,
  type SessionFilter,
} from "./sessions/SessionsTable";

const SEG_OPTIONS: { id: SessionFilter; label: string }[] = [
  { id: "all", label: "All" },
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
    Promise.all([
      api.sessionListAll(),
      api.projectList(),
      api.sessionWorktreeGroups().catch(() => null),
    ])
      .then(([ss, ps, groups]) => {
        if (!mountedRef.current || myToken !== tokenRef.current) return;
        setSessions(ss);
        setProjects(ps);
        setRepoGroups(groups);
        setLoading(false);
        // Drop the selection if it no longer exists.
        setSelectedPath((prev) =>
          prev && ss.some((s) => s.file_path === prev) ? prev : null,
        );
        // Drop the active repo id if the new groups don't contain it.
        // Id is `repo_root` for git-tracked repos, `label` for no-repo.
        setActiveRepo((prev) =>
          prev &&
          groups &&
          groups.some((g) => (g.repo_root ?? g.label) === prev)
            ? prev
            : null,
        );
      })
      .catch((e) => {
        if (!mountedRef.current || myToken !== tokenRef.current) return;
        setError(String(e));
        setLoading(false);
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
  // mention the word. Debounced + 2-char min inside the hook.
  const { hits: deepHits } = useSessionSearch(query, 50);
  const deepHitPaths = useMemo(
    () => new Set(deepHits.map((h) => h.file_path)),
    [deepHits],
  );
  const filteredByQuery = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return scoped;
    return scoped.filter((s) => {
      if (s.session_id.toLowerCase().startsWith(q)) return true;
      if (s.project_path.toLowerCase().includes(q)) return true;
      if ((s.first_user_prompt ?? "").toLowerCase().includes(q)) return true;
      if (s.models.some((m) => m.toLowerCase().includes(q))) return true;
      if ((s.git_branch ?? "").toLowerCase().includes(q)) return true;
      // Deep content hit from the backend search.
      if (deepHitPaths.has(s.file_path)) return true;
      return false;
    });
  }, [scoped, query, deepHitPaths]);

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

  const subtitle = (() => {
    if (error && sessions.length === 0) return "Couldn't load sessions.";
    const total = sessions.length;
    if (total === 0) return "No sessions yet. Run `claude` to start one.";
    const narrowed = query.trim() && filteredByQuery.length !== total;
    if (narrowed) {
      return `${filteredByQuery.length} of ${total} session${
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
            role="tablist"
            style={{
              display: "flex",
              gap: "var(--sp-2)",
              padding: "var(--sp-2)",
              background: "var(--bg-sunken)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
            }}
          >
            {SEG_OPTIONS.map((opt) => {
              const current = filter === opt.id;
              const count = counts[opt.id];
              return (
                <button
                  key={opt.id}
                  type="button"
                  role="tab"
                  aria-selected={current}
                  onClick={() => setFilter(opt.id)}
                  style={{
                    padding: "var(--sp-4) var(--sp-10)",
                    fontSize: "var(--fs-xs)",
                    fontWeight: 500,
                    color: current ? "var(--fg)" : "var(--fg-muted)",
                    background: current ? "var(--bg-raised)" : "transparent",
                    border: current
                      ? "var(--bw-hair) solid var(--line)"
                      : "var(--bw-hair) solid transparent",
                    borderRadius: "var(--r-1)",
                    letterSpacing: "var(--ls-wide)",
                    textTransform: "uppercase",
                    cursor: "pointer",
                  }}
                >
                  {opt.label} · {count}
                </button>
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
            <div
              style={{
                flex: 1,
                minWidth: 0,
                overflow: "auto",
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
                navigator.clipboard.writeText(s.session_id);
                setToast("Copied session id.");
              },
            },
            {
              label: "Copy project path",
              onClick: () => {
                navigator.clipboard.writeText(s.project_path);
                setToast("Copied project path.");
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
