import { useCallback, useEffect, useMemo, useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import { ContextMenu, type ContextMenuItem } from "../../components/ContextMenu";
import {
  BrandGithubMark,
  LiveStatusDot,
  liveDotTitle,
} from "../../components/primitives";
import { openUrl } from "@tauri-apps/plugin-opener";
import { fileManagerName } from "../../lib/platformLabels";
import { useAppState } from "../../providers/AppStateProvider";
import { useSessionLive } from "../../hooks/useSessionLive";
import type {
  ProjectDetail as ProjectDetailData,
  ProjectInfo,
  SessionRow,
} from "../../types";
import { classifyProject } from "./projectStatus";
import { formatRelativeTime, formatSize } from "./format";
import { MoveSessionModal } from "./MoveSessionModal";
import { PermissionPanel } from "./PermissionPanel";
import { ProjectEnvPanel } from "./ProjectEnvPanel";
import { sessionCostEstimate, formatUsd, usePriceTable } from "../../costs";

/**
 * Build the on-disk path of a session transcript given the containing
 * project's sanitized slug. Returns null if we don't yet know where CC
 * stores its config (AppStatus hasn't loaded) — callers then skip
 * "reveal" affordances that need the absolute path.
 */
function sessionFilePath(
  ccConfigDir: string | undefined,
  sanitizedName: string,
  sessionId: string,
): string | null {
  if (!ccConfigDir) return null;
  const joiner = ccConfigDir.endsWith("/") ? "" : "/";
  return `${ccConfigDir}${joiner}projects/${sanitizedName}/${sessionId}.jsonl`;
}

/**
 * Right-pane detail view for the selected project. Shows paths, size,
 * session count, memory files, plus a session list with a right-click
 * "Move to another project…" action per row.
 */
export function ProjectDetail({
  path,
  projects,
  refreshSignal,
  onRename,
  onMoved,
  onError,
  onOpenMaintenance,
  onOpenInConfig,
  onOpenSession,
  onBack,
}: {
  path: string;
  /** Live list of projects — powers the session-move target picker. */
  projects: ProjectInfo[];
  /** Bumped by the parent whenever external state changes mean this
   * pane's cached detail is stale — e.g. after a session moves out
   * of this project. The effect includes it as a dep so the refetch
   * fires even when `path` itself hasn't changed. */
  refreshSignal: number;
  onRename: (path: string) => void;
  /** Fires after a session move succeeds so the caller can refresh. */
  onMoved: () => void;
  /** Optional error sink for fire-and-forget ops (e.g. Reveal in Finder
   * when the native open fails). Parent typically wires this to its
   * toast state. Missing → errors are logged and swallowed. */
  onError?: (msg: string) => void;
  /** When set, empty-project hints get a clickable "Go to Maintenance"
   * nudge so the user doesn't have to navigate manually (G8). */
  onOpenMaintenance?: () => void;
  /**
   * Jump to the Config section anchored on this project. Wired by the
   * shell. Button is only rendered when this prop is present AND the
   * project is reachable — an unreachable source path can't be walked
   * by Config's scanner.
   */
  onOpenInConfig?: (path: string) => void;
  /**
   * Open a session's transcript inline. The shell owns the master-
   * detail toggle inside the Projects section's Sessions tab — when
   * this callback fires with a resolved transcript path, the shell
   * swaps this ProjectDetail for the SessionDetail viewer. Omitted
   * in embedded contexts (e.g. tests) where the master-detail swap
   * isn't wired.
   */
  onOpenSession?: (transcriptPath: string) => void;
  /** Single-pane mode: render a Back button so the user can return to
   * the project list on narrow windows. When omitted the header reads
   * as detail-first (the surrounding layout is already a split). */
  onBack?: () => void;
}) {
  const [detail, setDetail] = useState<ProjectDetailData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<
    { x: number; y: number; sessionId: string } | null
  >(null);
  const [moveTarget, setMoveTarget] = useState<string | null>(null);
  const { status: appStatus } = useAppState();
  // Token + model data for per-session cost. `null` until the first
  // `session_list_all` resolves; `[]` when the index has no rows for
  // this project. Failure leaves it `null` so the UI falls back to
  // size-only rows rather than rendering "$0.00" misleadingly.
  const [sessionRows, setSessionRows] = useState<SessionRow[] | null>(null);
  const { table: priceTable } = usePriceTable();

  const onSessionContextMenu = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY, sessionId });
    },
    [],
  );
  const onSessionMenuButton = useCallback(
    (e: React.MouseEvent, sessionId: string) => {
      e.stopPropagation();
      // Anchor the menu to the button's bottom-left so the menu
      // appears predictably below the row rather than wherever the
      // cursor happened to be.
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      setCtxMenu({ x: r.left, y: r.bottom + 2, sessionId });
    },
    [],
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .projectShow(path)
      .then((d) => {
        if (!cancelled) {
          setDetail(d);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [path, refreshSignal]);

  // Pull the session index once per (project, refresh). Filtering by
  // slug here keeps the join exact even when an old transcript's
  // `cwd` differs slightly from the canonicalized `original_path`
  // (case-folded on macOS, trailing-slash differences, etc.).
  //
  // Clear `sessionRows` synchronously before each fetch so the heading
  // total + per-row costs never render against a previous project's
  // data during the in-flight window. The useMemo below treats `null`
  // as "no cost yet" — rows render without a $ trailer, which is the
  // honest state during a refetch.
  const sanitizedName = detail?.info.sanitized_name;
  useEffect(() => {
    if (!sanitizedName) return;
    setSessionRows(null);
    let cancelled = false;
    api
      .sessionListAll()
      .then((rows) => {
        if (cancelled) return;
        setSessionRows(rows.filter((r) => r.slug === sanitizedName));
      })
      .catch(() => {
        if (cancelled) return;
        setSessionRows(null);
      });
    return () => {
      cancelled = true;
    };
  }, [sanitizedName, refreshSignal]);

  // Per-session cost map keyed by session_id, plus a project total.
  // `null` cost = priced model unknown for that session — render
  // nothing rather than $0.00. `null` total = no priceable sessions.
  const { costBySessionId, sessionTotal } = useMemo(() => {
    const map = new Map<string, number | null>();
    if (!sessionRows) {
      return { costBySessionId: map, sessionTotal: null as number | null };
    }
    let sum = 0;
    let priced = 0;
    for (const r of sessionRows) {
      const c = sessionCostEstimate(priceTable, r.models, r.tokens);
      map.set(r.session_id, c);
      if (c != null) {
        sum += c;
        priced += 1;
      }
    }
    return {
      costBySessionId: map,
      sessionTotal: priced > 0 ? sum : null,
    };
  }, [sessionRows, priceTable]);

  if (loading && !detail) {
    return (
      <main className="content">
        <div className="skeleton-container">
          <div className="skeleton skeleton-header" />
          <div className="skeleton skeleton-card" />
        </div>
      </main>
    );
  }
  if (error) {
    return (
      <main className="content">
        <div className="empty">
          <h2>Couldn't load project</h2>
          <p className="muted mono">{error}</p>
        </div>
      </main>
    );
  }
  if (!detail) return <main className="content" />;

  const { info, sessions } = detail;
  const status = classifyProject(info);
  const noContent = info.session_count === 0 && info.memory_file_count === 0;

  return (
    <main className="content project-detail">
      <header className="project-detail-header">
        <div className="project-detail-title">
          {onBack && (
            <button
              type="button"
              className="icon-btn"
              onClick={onBack}
              aria-label="Back to project list"
              title="Back to project list"
            >
              <Icon name="arrow-left" size={14} />
            </button>
          )}
          <h2 className="selectable" title={info.original_path}>
            {info.original_path.split("/").filter(Boolean).pop() ??
              info.sanitized_name}
          </h2>
          {info.pr && (
            <button
              type="button"
              onClick={() => {
                if (info.pr) void openUrl(info.pr.url).catch(() => {});
              }}
              title={`PR #${info.pr.number} — ${info.pr.state}`}
              aria-label={`Open pull request #${info.pr.number}`}
              style={{
                background: "transparent",
                border: "none",
                padding: 0,
                cursor: "pointer",
                color: info.pr.state === "open"
                  ? "var(--accent)"
                  : "var(--fg-muted)",
                lineHeight: 0,
                display: "inline-flex",
                alignItems: "center",
                gap: "var(--sp-4)",
              }}
            >
              <BrandGithubMark size={16} />
              <span style={{ fontSize: "var(--fs-xs)" }}>#{info.pr.number}</span>
            </button>
          )}
          {status === "orphan" && (
            <span className="project-tag orphan" title="source directory does not exist">
              orphan
            </span>
          )}
          {status === "unreachable" && (
            <span className="project-tag unreachable" title="source lives on an unmounted volume or permission-denied path">
              unreachable
            </span>
          )}
          {status === "empty" && (
            <span className="project-tag empty" title="CC project dir has no sessions or memory files">
              empty
            </span>
          )}
        </div>
        <div className="project-detail-actions">
          <button
            type="button"
            className="btn"
            title={`Reveal this project's directory in ${fileManagerName(appStatus?.platform)}`}
            onClick={() => {
              api.revealInFinder(info.original_path).catch((e) => {
                const msg = `Couldn't reveal: ${e}`;
                if (onError) onError(msg);
                else console.error(msg);
              });
            }}
          >
            <Icon name="folder-open" />{fileManagerName(appStatus?.platform)}
          </button>
          <button type="button" className="btn" title="Rename this project"
            onClick={() => onRename(info.original_path)}>
            <Icon name="pencil" />Rename…
          </button>
          {onOpenInConfig && status !== "unreachable" && status !== "orphan" && (
            <button
              type="button"
              className="btn"
              title="View this project's Claude Code config — merged settings, MCP, agents, memory"
              onClick={() => onOpenInConfig(info.original_path)}
            >
              <Icon name="file-code" />Config
            </button>
          )}
        </div>
      </header>

      {status === "unreachable" && (
        <div className="project-hint unreachable" role="status">
          <Icon name="wifi-off" size={14} />
          <span>
            Source path can't be checked right now (unmounted volume or
            permission-denied ancestor). Mount the drive and click Refresh
            to re-classify.
          </span>
        </div>
      )}

      <section className="detail-grid">
        <span className="detail-label">Path</span>
        <span className="detail-value mono selectable">
          {info.original_path} <CopyButton text={info.original_path} />
        </span>
        <span className="detail-label">Size</span>
        <span className="detail-value">{formatSize(info.total_size_bytes)}</span>
        {info.last_modified_ms != null && (
          <>
            <span className="detail-label">Last touched</span>
            <span className="detail-value">
              {formatRelativeTime(info.last_modified_ms)}
            </span>
          </>
        )}
        {info.session_count > 0 && (
          <>
            <span className="detail-label">Sessions</span>
            <span className="detail-value">{info.session_count}</span>
          </>
        )}
        {info.memory_file_count > 0 && (
          <>
            <span className="detail-label">Memory</span>
            <span className="detail-value">
              {info.memory_file_count} file{info.memory_file_count === 1 ? "" : "s"}
            </span>
          </>
        )}
      </section>

      {status !== "orphan" && status !== "unreachable" && (
        <>
          {/* `key` remounts each panel on project switch — fresh state,
              effects re-run, in-flight async from the prior project is
              cleaned up rather than leaking into the new one. */}
          <PermissionPanel
            key={info.original_path}
            projectPath={info.original_path}
            onError={onError}
          />
          <ProjectEnvPanel
            key={info.original_path}
            projectPath={info.original_path}
            onError={onError}
          />
        </>
      )}

      {noContent && status !== "alive" && status !== "unreachable" && (
        <div className="project-hint cleanup" role="status">
          <Icon name="info" size={14} />
          <span>No sessions or memory. This project is a cleanup candidate.</span>
          {onOpenMaintenance && (
            <button
              type="button"
              className="btn"
              onClick={onOpenMaintenance}
              title="Open Maintenance to clean orphan projects"
            >
              Go to Maintenance
            </button>
          )}
        </div>
      )}

      {sessions.length > 0 && (
        <SessionListPane
          sessions={sessions}
          costBySessionId={costBySessionId}
          totalCost={sessionTotal}
          onOpen={
            onOpenSession
              ? (sid) => {
                  const p = sessionFilePath(
                    appStatus?.cc_config_dir,
                    info.sanitized_name,
                    sid,
                  );
                  if (p) onOpenSession(p);
                }
              : undefined
          }
          onContextMenu={onSessionContextMenu}
          onMenuButton={onSessionMenuButton}
        />
      )}

      {ctxMenu &&
        (() => {
          const transcriptPath = sessionFilePath(
            appStatus?.cc_config_dir,
            info.sanitized_name,
            ctxMenu.sessionId,
          );
          const canOpenInConfig =
            !!onOpenInConfig &&
            classifyProject(info) !== "orphan" &&
            classifyProject(info) !== "unreachable";
          const items: ContextMenuItem[] = [
            {
              label: "Move to another project…",
              onClick: () => setMoveTarget(ctxMenu.sessionId),
            },
            ...(canOpenInConfig
              ? [
                  { label: "", separator: true, onClick: () => {} },
                  {
                    label: "Open project in Config",
                    onClick: () => onOpenInConfig!(info.original_path),
                  },
                ] as ContextMenuItem[]
              : []),
            ...(transcriptPath
              ? ([
                  { label: "", separator: true, onClick: () => {} },
                  {
                    label: "Reveal session in Finder",
                    onClick: () => {
                      api.revealInFinder(transcriptPath).catch((e) => {
                        const msg = `Couldn't reveal: ${e}`;
                        if (onError) onError(msg);
                        else console.error(msg);
                      });
                    },
                  },
                  {
                    label: "Copy session file path",
                    onClick: () => {
                      navigator.clipboard.writeText(transcriptPath);
                    },
                  },
                ] as ContextMenuItem[])
              : []),
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Copy session ID",
              onClick: () => {
                navigator.clipboard.writeText(ctxMenu.sessionId);
              },
            },
          ];
          return (
            <ContextMenu
              x={ctxMenu.x}
              y={ctxMenu.y}
              items={items}
              onClose={() => setCtxMenu(null)}
            />
          );
        })()}

      {moveTarget && (
        <MoveSessionModal
          sessionId={moveTarget}
          fromCwd={info.original_path}
          projects={projects}
          onClose={() => setMoveTarget(null)}
          onCompleted={() => {
            setMoveTarget(null);
            onMoved();
          }}
        />
      )}
    </main>
  );
}

const PAGE_SIZE = 20;

/**
 * Session list with id-prefix search and incremental pagination.
 *
 * The previous implementation hard-capped at 20 rows with no recourse —
 * so sessions 21+ were unreachable from the GUI. This version renders
 * the first PAGE_SIZE by default, offers a "Show more" button that
 * grows the window, and lets the user filter by session-id prefix to
 * drill straight to a specific transcript.
 *
 * Rows are keyboard-reachable (role=option + tabIndex + Enter/Space
 * opens the actions menu) so this satisfies the design-rules
 * accessibility floor the old implementation silently missed.
 */
function SessionListPane({
  sessions,
  costBySessionId,
  totalCost,
  onOpen,
  onContextMenu,
  onMenuButton,
}: {
  sessions: ProjectDetailData["sessions"];
  /**
   * Hypothetical API-rate cost per session, keyed by session_id.
   * Missing key = not yet computed (or session index hasn't reached
   * this transcript yet) → row shows no cost. `null` value = the
   * session's models couldn't be priced → row also shows no cost
   * rather than $0.00.
   */
  costBySessionId?: Map<string, number | null>;
  /** Sum of all priceable sessions in this project. `null` when no
   *  session was priceable; render nothing rather than $0.00. */
  totalCost?: number | null;
  /**
   * Click-to-open: fires when the user picks a session row to view
   * its transcript inline. `null` disables the primary click (row
   * keeps working as a target for right-click / kebab only).
   */
  onOpen?: (sid: string) => void;
  onContextMenu: (e: React.MouseEvent, sid: string) => void;
  onMenuButton: (e: React.MouseEvent, sid: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(PAGE_SIZE);

  // Subscribe once; useSessionLive is a singleton store, so calling it
  // from N rendered ProjectDetails costs one backend listener total.
  const liveAll = useSessionLive();
  const liveBySessionId = useMemo(() => {
    const m = new Map<string, (typeof liveAll)[number]>();
    for (const s of liveAll) m.set(s.session_id, s);
    return m;
  }, [liveAll]);

  const q = query.trim().toLowerCase();
  const filtered = q
    ? sessions.filter((s) => s.session_id.toLowerCase().includes(q))
    : sessions;
  const visible = filtered.slice(0, limit);
  const hiddenCount = Math.max(0, filtered.length - visible.length);

  const handleKeyDown = (e: React.KeyboardEvent, sid: string) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      // Primary key action opens the transcript when a handler is
      // available; falls back to the actions menu (legacy behavior)
      // so the old keyboard flow still works in contexts that don't
      // wire opening (tests, Storybook).
      if (onOpen) onOpen(sid);
      else onMenuButton(e as unknown as React.MouseEvent, sid);
    }
  };

  return (
    <section className="detail-section">
      <div className="session-list-header">
        <h3>
          Sessions · {sessions.length}
          {typeof totalCost === "number" && totalCost > 0 && (
            <>
              {" · "}
              <span
                className="muted"
                title="Sum of hypothetical Anthropic API cost across every session in this project. Real billing depends on your plan."
              >
                {formatUsd(totalCost)} at API rates
              </span>
            </>
          )}
        </h3>
        <input
          type="search"
          className="session-filter mono"
          placeholder="Filter by id prefix"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setLimit(PAGE_SIZE);
          }}
          aria-label="Filter sessions by id prefix"
        />
      </div>
      {filtered.length === 0 ? (
        <p className="muted small">No sessions match that filter.</p>
      ) : (
        <ul className="session-list" role="listbox" aria-label="Sessions">
          {visible.map((s) => {
            const cost = costBySessionId?.get(s.session_id);
            const live = liveBySessionId.get(s.session_id);
            return (
            <li
              key={s.session_id}
              className="session-row"
              role="option"
              aria-selected={false}
              tabIndex={0}
              onClick={() => onOpen?.(s.session_id)}
              onContextMenu={(e) => onContextMenu(e, s.session_id)}
              onKeyDown={(e) => handleKeyDown(e, s.session_id)}
              style={onOpen ? { cursor: "pointer" } : undefined}
            >
              <div className="session-row-text">
                <span
                  className="session-row-name mono"
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  {live && (
                    <LiveStatusDot
                      status={live.status}
                      errored={live.errored}
                      title={liveDotTitle(live)}
                    />
                  )}
                  {s.session_id.slice(0, 8)}
                </span>
                <span className="session-row-meta">
                  {formatSize(s.file_size)}
                  {s.last_modified_ms != null && (
                    <>{" · "}{formatRelativeTime(s.last_modified_ms)}</>
                  )}
                  {typeof cost === "number" && cost > 0 && (
                    <>{" · "}{formatUsd(cost)}</>
                  )}
                </span>
              </div>
              <button
                type="button"
                className="session-row-menu-btn"
                aria-label="Session actions"
                title="Actions"
                onClick={(e) => {
                  e.stopPropagation();
                  onMenuButton(e, s.session_id);
                }}
              >
                <Icon name="more-vertical" size={12} />
              </button>
            </li>
            );
          })}
        </ul>
      )}
      {hiddenCount > 0 && (
        <div className="session-list-more">
          <button
            type="button"
            className="btn"
            onClick={() => setLimit((n) => n + PAGE_SIZE)}
          >
            Show {Math.min(hiddenCount, PAGE_SIZE)} more
          </button>
          <span className="muted small">
            {hiddenCount} more hidden
          </span>
        </div>
      )}
    </section>
  );
}

