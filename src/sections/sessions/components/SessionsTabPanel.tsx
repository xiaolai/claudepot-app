import type { MouseEvent } from "react";
import { Button } from "../../../components/primitives/Button";
import { FilterChip } from "../../../components/primitives/FilterChip";
import { Glyph } from "../../../components/primitives/Glyph";
import { Input } from "../../../components/primitives/Input";
import { NF } from "../../../icons";
import type {
  ProjectInfo,
  RepositoryGroup,
  SessionRow,
} from "../../../types";
import { RepoFilterStrip } from "../RepoFilterStrip";
import { SessionDetail } from "../SessionDetail";
import {
  SessionsTable,
  type SessionFilter,
} from "../SessionsTable";

/**
 * Toggleable status chips: each flips the filter between "all" and
 * its own value. Two chips active at once is not a supported state —
 * picking one deselects the other (mutual exclusion preserves the
 * existing `SessionFilter` enum shape).
 */
const FILTER_CHIPS: { id: Exclude<SessionFilter, "all">; label: string }[] = [
  { id: "errors", label: "Errors" },
  { id: "sidechain", label: "Agents" },
];

/**
 * Tabpanel for the "Sessions" view. Owns the layout of repo strip +
 * filter row + (error pane | table+detail). Rendered inside a
 * `display: contents` wrapper in `SessionsSection` so its children
 * remain direct flex siblings of the section root.
 *
 * Pure presentational — every dependency comes in as a prop. The
 * parent owns selection / query / filter state, the data hook, and
 * the toast/move modal surfaces.
 */
export function SessionsTabPanel({
  showTable,
  showDetail,
  splitView,
  repoGroups,
  activeRepo,
  setActiveRepo,
  query,
  setQuery,
  filter,
  setFilter,
  counts,
  loading,
  error,
  sessions,
  filteredByQuery,
  searchSnippets,
  selectedPath,
  setSelectedPath,
  projects,
  detailRefreshSignal,
  setDetailRefreshSignal,
  onContextMenu,
  onRefresh,
  setToast,
}: {
  showTable: boolean;
  showDetail: boolean;
  splitView: boolean;
  repoGroups: RepositoryGroup[] | null;
  activeRepo: string | null;
  setActiveRepo: (repoId: string | null) => void;
  query: string;
  setQuery: (q: string) => void;
  filter: SessionFilter;
  setFilter: (f: SessionFilter) => void;
  counts: Record<SessionFilter, number>;
  loading: boolean;
  error: string | null;
  sessions: SessionRow[];
  filteredByQuery: SessionRow[];
  searchSnippets: Map<string, string>;
  selectedPath: string | null;
  setSelectedPath: (path: string | null) => void;
  projects: ProjectInfo[];
  detailRefreshSignal: number;
  setDetailRefreshSignal: React.Dispatch<React.SetStateAction<number>>;
  onContextMenu: (e: MouseEvent, s: SessionRow) => void;
  onRefresh: () => void;
  setToast: (msg: string) => void;
}) {
  return (
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
            style={{ display: "flex", gap: "var(--sp-6)" }}
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
          <Button variant="solid" onClick={onRefresh}>
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
                onContextMenu={onContextMenu}
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
                initialSearch={query.trim() ? query.trim() : undefined}
                onMoved={() => {
                  setDetailRefreshSignal((n) => n + 1);
                  onRefresh();
                }}
                onError={(msg) => setToast(msg)}
                onBack={splitView ? undefined : () => setSelectedPath(null)}
              />
            </aside>
          )}
        </div>
      )}
    </div>
  );
}
