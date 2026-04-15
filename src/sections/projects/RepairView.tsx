import { useEffect, useState } from "react";
import { ArrowLeft, Wrench } from "@phosphor-icons/react";
import { api } from "../../api";
import type { JournalEntry, JournalStatus } from "../../types";

const STATUS_COPY: Record<JournalStatus, string> = {
  running: "running",
  pending: "pending",
  stale: "stale ≥24h",
  abandoned: "abandoned",
};

/**
 * Read-only view of pending rename journals. Actions (Resume /
 * Rollback / Abandon / Break-lock / Inspect) land in Step 4 — this
 * view establishes the layout + polling plumbing without shipping
 * mutation yet.
 */
export function RepairView({ onBack }: { onBack: () => void }) {
  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .repairList()
      .then((es) => {
        if (!cancelled) {
          setEntries(es);
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
  }, []);

  return (
    <main className="content repair-view">
      <header className="repair-header">
        <button
          type="button"
          className="icon-btn"
          onClick={onBack}
          aria-label="Back to Projects"
          title="Back to Projects"
        >
          <ArrowLeft />
        </button>
        <h2>
          <Wrench /> Repair
        </h2>
      </header>

      {loading && (
        <div className="skeleton-container">
          <div className="skeleton skeleton-card" />
        </div>
      )}

      {error && (
        <div className="banner warn" role="alert">
          <div>
            <strong>Couldn't load repair queue.</strong>{" "}
            <span className="mono">{error}</span>
          </div>
        </div>
      )}

      {!loading && !error && entries.length === 0 && (
        <div className="empty">
          <Wrench size={32} weight="thin" />
          <h2>All clear</h2>
          <p className="muted">No pending rename journals.</p>
        </div>
      )}

      {!loading && entries.length > 0 && (
        <ul className="repair-list">
          {entries.map((e) => (
            <li
              key={e.id}
              className={`repair-entry status-${e.status}`}
              aria-label={`Journal ${e.id} — ${STATUS_COPY[e.status]}`}
            >
              <div className="repair-entry-head">
                <span className={`tag ${statusClass(e.status)}`}>
                  {STATUS_COPY[e.status]}
                </span>
                <span className="mono small muted">{e.id}</span>
              </div>
              <div className="repair-entry-paths">
                <span className="mono small selectable">{e.old_path}</span>
                <span className="muted"> → </span>
                <span className="mono small selectable">{e.new_path}</span>
              </div>
              <div className="repair-entry-meta muted small">
                started {e.started_at} · phases [
                {e.phases_completed.join(", ") || "none"}]
              </div>
              {e.last_error && (
                <div className="repair-entry-error bad small">
                  last error: {e.last_error}
                </div>
              )}
              {/* Actions come in Step 4. */}
              <div className="repair-entry-actions muted small">
                Actions (Resume, Rollback, Abandon) coming in the next step.
              </div>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}

function statusClass(s: JournalStatus): string {
  switch (s) {
    case "running":
      return "ok";
    case "pending":
      return "";
    case "stale":
      return "warn";
    case "abandoned":
      return "muted";
  }
}
