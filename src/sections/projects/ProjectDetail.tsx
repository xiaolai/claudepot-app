import { useEffect, useState } from "react";
import { Folder, PencilSimple, Warning } from "@phosphor-icons/react";
import { api } from "../../api";
import { CopyButton } from "../../components/CopyButton";
import type { ProjectDetail as ProjectDetailData } from "../../types";

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
  return (
    <main className="content project-detail">
      <header className="project-detail-header">
        <div className="project-detail-title">
          <Folder />
          <h2 className="selectable" title={info.original_path}>
            {info.original_path.split("/").filter(Boolean).pop() ??
              info.sanitized_name}
          </h2>
          {info.is_orphan && (
            <span className="tag warn" title="source directory not found">
              <Warning size={11} weight="bold" /> orphan
            </span>
          )}
        </div>
        <div className="project-detail-actions">
          <button
            type="button"
            className="primary"
            title="Rename this project"
            onClick={() => onRename(info.original_path)}
          >
            <PencilSimple /> Rename…
          </button>
        </div>
      </header>

      <section className="detail-grid">
        <div className="detail-row">
          <span className="detail-label">Original path</span>
          <span className="detail-value mono selectable">
            {info.original_path}
            <CopyButton text={info.original_path} />
          </span>
        </div>
        <div className="detail-row">
          <span className="detail-label">Sanitized name</span>
          <span className="detail-value mono selectable">
            {info.sanitized_name}
          </span>
        </div>
        <div className="detail-row">
          <span className="detail-label">Size</span>
          <span className="detail-value">
            {formatSize(info.total_size_bytes)}
          </span>
        </div>
        <div className="detail-row">
          <span className="detail-label">Sessions</span>
          <span className="detail-value">{info.session_count}</span>
        </div>
        <div className="detail-row">
          <span className="detail-label">Memory files</span>
          <span className="detail-value">{info.memory_file_count}</span>
        </div>
      </section>

      {memory_files.length > 0 && (
        <section className="detail-section">
          <h3>Memory files</h3>
          <ul className="detail-list mono small">
            {memory_files.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </section>
      )}

      {sessions.length > 0 && (
        <section className="detail-section">
          <h3>Sessions ({sessions.length})</h3>
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

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
