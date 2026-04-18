import { useEffect, useState } from "react";
import { Pencil, WifiOff, Trash2, Info } from "lucide-react";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import type { ProjectDetail as ProjectDetailData } from "../../types";
import { classifyProject } from "./projectStatus";
import { formatSize } from "./format";

/**
 * Right-pane detail view for the selected project. Shows paths, size,
 * session count, memory files. The Rename button is a stub until
 * Step 5 (rename modal + live dry-run).
 */
export function ProjectDetail({
  path,
  onRename,
}: {
  path: string;
  onRename: (path: string) => void;
}) {
  const [detail, setDetail] = useState<ProjectDetailData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

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
  }, [path]);

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
          <button type="button" title="Rename this project"
            onClick={() => onRename(info.original_path)}>
            <Pencil size={14} /> Rename…
          </button>
        </div>
      </header>

      {status === "unreachable" && (
        <div className="project-hint unreachable" role="status">
          <WifiOff size={14} />
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
        <span className="detail-label">Key</span>
        <span className="detail-value mono selectable">{info.sanitized_name}</span>
        <span className="detail-label">Size</span>
        <span className="detail-value">{formatSize(info.total_size_bytes)}</span>
        <span className="detail-label">Sessions</span>
        <span className="detail-value">{info.session_count}</span>
        <span className="detail-label">Memory</span>
        <span className="detail-value">{info.memory_file_count} file{info.memory_file_count === 1 ? "" : "s"}</span>
      </section>

      {noContent && status === "alive" && (
        <div className="project-hint cleanup" role="status">
          <Trash2 size={14} />
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
          <Info size={14} />
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
          <ul className="detail-list mono small">
            {sessions.slice(0, 20).map((s) => (
              <li key={s.session_id}>
                <span className="muted">{s.session_id.slice(0, 8)}</span> —{" "}
                {formatSize(s.file_size)}
              </li>
            ))}
            {sessions.length > 20 && (
              <li className="muted">
                … {sessions.length - 20} more not shown
              </li>
            )}
          </ul>
        </section>
      )}
    </main>
  );
}

