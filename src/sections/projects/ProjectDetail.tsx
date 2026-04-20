import { useCallback, useEffect, useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import { ContextMenu, type ContextMenuItem } from "../../components/ContextMenu";
import type { ProjectDetail as ProjectDetailData, ProjectInfo } from "../../types";
import { classifyProject } from "./projectStatus";
import { formatRelativeTime, formatSize } from "./format";
import { MoveSessionModal } from "./MoveSessionModal";

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
}) {
  const [detail, setDetail] = useState<ProjectDetailData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<
    { x: number; y: number; sessionId: string } | null
  >(null);
  const [moveTarget, setMoveTarget] = useState<string | null>(null);

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

  const { info, sessions, memory_files } = detail;
  const status = classifyProject(info);
  const noContent = info.session_count === 0 && info.memory_file_count === 0;

  return (
    <main className="content project-detail">
      <header className="project-detail-header">
        <div className="project-detail-title">
          <h2 className="selectable" title={info.original_path}>
            {info.original_path.split("/").filter(Boolean).pop() ??
              info.sanitized_name}
          </h2>
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
            title="Reveal this project's directory in the native file manager"
            onClick={() => {
              api.revealInFinder(info.original_path).catch((e) => {
                const msg = `Couldn't reveal: ${e}`;
                if (onError) onError(msg);
                else console.error(msg);
              });
            }}
          >
            <Icon name="folder-open" size={14} /> Open in Finder
          </button>
          <button type="button" className="btn" title="Rename this project"
            onClick={() => onRename(info.original_path)}>
            <Icon name="pencil" size={14} /> Rename…
          </button>
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

      {noContent && status === "alive" && (
        <div className="project-hint cleanup" role="status">
          <Icon name="trash-2" size={14} />
          <span>
            No sessions or memory files.{" "}
            {info.total_size_bytes > 4096
              ? `${formatSize(info.total_size_bytes)} of CC internal state — consider cleaning.`
              : "This project can be safely cleaned."}
          </span>
        </div>
      )}

      {noContent && status !== "alive" && status !== "unreachable" && (
        <div className="project-hint cleanup" role="status">
          <Icon name="info" size={14} />
          <span>No sessions or memory. This project is a cleanup candidate.</span>
        </div>
      )}

      {memory_files.length > 0 && (
        <section className="detail-section">
          <h3>Memory</h3>
          <ul className="detail-list mono small">
            {memory_files.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </section>
      )}

      {sessions.length > 0 && (
        <section className="detail-section">
          <h3>Sessions · {sessions.length}</h3>
          <ul className="session-list" role="list">
            {sessions.slice(0, 20).map((s) => (
              <li
                key={s.session_id}
                className="session-row"
                onContextMenu={(e) => onSessionContextMenu(e, s.session_id)}
              >
                <div className="session-row-text">
                  <span className="session-row-name mono">
                    {s.session_id.slice(0, 8)}
                  </span>
                  <span className="session-row-meta">
                    {formatSize(s.file_size)}
                    {s.last_modified_ms != null && (
                      <>{" · "}{formatRelativeTime(s.last_modified_ms)}</>
                    )}
                  </span>
                </div>
                <button
                  type="button"
                  className="session-row-menu-btn"
                  aria-label="Session actions"
                  title="Actions"
                  onClick={(e) => onSessionMenuButton(e, s.session_id)}
                >
                  <Icon name="more-vertical" size={12} />
                </button>
              </li>
            ))}
            {sessions.length > 20 && (
              <li className="session-row-more muted">
                … {sessions.length - 20} more not shown
              </li>
            )}
          </ul>
        </section>
      )}

      {ctxMenu &&
        (() => {
          const items: ContextMenuItem[] = [
            {
              label: "Move to another project…",
              onClick: () => setMoveTarget(ctxMenu.sessionId),
            },
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

