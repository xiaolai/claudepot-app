// Shared Memory section (WI-007 of shared-memory.md).
//
// Cross-harness search across indexed Claude + Codex transcripts,
// durable memories, and design decisions. The MCP server exposes
// the same shapes to agents; this surface is for the human.
//
// Three tabs (Search / Memories / Decisions). Each tab is
// independent — the search is the primary verb; memories and
// decisions are inspection / curation surfaces.

import { useCallback, useEffect, useMemo, useState } from "react";
import { sharedMemoryApi } from "../api/sharedMemory";
import type {
  Decision,
  DecisionStatus,
  Memory,
  MemoryKind,
  MemoryScope,
  SearchHit,
} from "../api/sharedMemory";
import { Button } from "../components/primitives/Button";
import { Glyph } from "../components/primitives/Glyph";
import { Input } from "../components/primitives/Input";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { Tag } from "../components/primitives/Tag";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";

type Tab = "search" | "memories" | "decisions";

const TABS: { id: Tab; label: string }[] = [
  { id: "search", label: "Search" },
  { id: "memories", label: "Memories" },
  { id: "decisions", label: "Decisions" },
];

export function SharedMemorySection() {
  const [tab, setTab] = useState<Tab>("search");

  // WAI-ARIA tabs pattern: Left/Right move selection with wrap-around
  // and focus follows. Required companion to TabButton's roving
  // tabIndex — without it, inactive tabs (tabIndex={-1}) would be
  // keyboard-unreachable.
  const onTablistKeyDown = (e: React.KeyboardEvent<HTMLElement>) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const delta = e.key === "ArrowRight" ? 1 : -1;
    const i = TABS.findIndex((t) => t.id === tab);
    const next = TABS[(i + delta + TABS.length) % TABS.length].id;
    setTab(next);
    document.getElementById(`shared-memory-tab-${next}`)?.focus();
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <ScreenHeader
        title="Shared Memory"
        subtitle="Cross-harness durable memory + indexed transcript search"
      />
      <nav
        role="tablist"
        aria-label="Shared memory tabs"
        onKeyDown={onTablistKeyDown}
        style={{
          display: "flex",
          gap: "var(--sp-16)",
          padding: "0 var(--sp-24)",
          borderBottom: "var(--sp-px) solid var(--line)",
        }}
      >
        {TABS.map((t) => (
          <TabButton
            key={t.id}
            id={`shared-memory-tab-${t.id}`}
            panelId={`shared-memory-panel-${t.id}`}
            active={tab === t.id}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </TabButton>
        ))}
      </nav>
      <div
        role="tabpanel"
        id={`shared-memory-panel-${tab}`}
        aria-labelledby={`shared-memory-tab-${tab}`}
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-24)",
        }}
      >
        {tab === "search" && <SearchTab />}
        {tab === "memories" && <MemoriesTab />}
        {tab === "decisions" && <DecisionsTab />}
      </div>
    </div>
  );
}

// Mirrors the canonical SectionTab ARIA contract
// (src/sections/sessions/components/SectionTab.tsx): id +
// aria-controls wired to the tabpanel, roving tabIndex (the tablist
// above supplies the arrow-key navigation that keeps inactive tabs
// reachable), and the design system's `pm-focus` ring. Only the
// visuals stay on this section's underline style.
function TabButton({
  id,
  panelId,
  active,
  onClick,
  children,
}: {
  id: string;
  panelId: string;
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      id={id}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={panelId}
      tabIndex={active ? 0 : -1}
      className="pm-focus"
      onClick={onClick}
      style={{
        padding: "var(--sp-12) 0",
        marginBottom: "calc(-1 * var(--sp-px))",
        borderBottom: active
          ? "var(--sp-2) solid var(--accent)"
          : "var(--sp-2) solid transparent",
        background: "transparent",
        color: active ? "var(--fg)" : "var(--fg-muted)",
        font: "inherit",
        fontWeight: active ? 600 : 400,
        cursor: "pointer",
      }}
    >
      {children}
    </button>
  );
}

// ─── Search tab ────────────────────────────────────────────────

function SearchTab() {
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<
    "all" | "claude_code" | "codex"
  >("all");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [body, setBody] = useState<string | null>(null);

  const runSearch = useCallback(async () => {
    setLoading(true);
    setErr(null);
    setExpanded(null);
    setBody(null);
    try {
      const res = await sharedMemoryApi.search({
        query: query.trim(),
        source_kind: sourceFilter === "all" ? null : sourceFilter,
        limit: 25,
      });
      setHits(res.hits);
      setHasMore(res.has_more);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [query, sourceFilter]);

  const expand = useCallback(
    async (hit: SearchHit) => {
      if (expanded === hit.exchange_id) {
        setExpanded(null);
        setBody(null);
        return;
      }
      setExpanded(hit.exchange_id);
      setBody(null);
      try {
        const r = await sharedMemoryApi.readLocator({
          file_path: hit.file_path,
          exchange_id: hit.exchange_id,
          max_bytes: 8 * 1024,
        });
        setBody(r.body);
      } catch (e) {
        setBody(`error: ${e}`);
      }
    },
    [expanded],
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch();
        }}
        style={{ display: "flex", gap: "var(--sp-8)" }}
      >
        <Input
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          placeholder='e.g. "rate limiter" or sk-ant-oat01'
          aria-label="Search query"
          style={{ flex: 1 }}
        />
        <select
          value={sourceFilter}
          onChange={(e) =>
            setSourceFilter(e.currentTarget.value as typeof sourceFilter)
          }
          aria-label="Source filter"
          style={{
            padding: "0 var(--sp-8)",
            background: "var(--bg-raised)",
            color: "var(--fg)",
            border: "var(--sp-px) solid var(--line)",
            borderRadius: "var(--r-2)",
            font: "inherit",
          }}
        >
          <option value="all">All sources</option>
          <option value="claude_code">Claude Code</option>
          <option value="codex">Codex</option>
        </select>
        <Button
          type="submit"
          variant="solid"
          glyph={NF.search}
          disabled={!query.trim() || loading}
        >
          Search
        </Button>
      </form>

      {err && (
        <div style={{ color: "var(--danger)" }}>{err}</div>
      )}

      {!loading && hits.length === 0 && !err && (
        <SectionLabel>
          {query.trim() ? "No matches." : "Enter a query to search."}
        </SectionLabel>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        {hits.map((hit) => (
          <article
            key={hit.exchange_id}
            style={{
              border: "var(--sp-px) solid var(--line)",
              borderRadius: "var(--r-3)",
              padding: "var(--sp-12)",
              background: "var(--bg-raised)",
            }}
          >
            <header
              style={{
                display: "flex",
                gap: "var(--sp-8)",
                alignItems: "center",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-muted)",
                marginBottom: "var(--sp-6)",
              }}
            >
              <Tag>{hit.source_kind === "codex" ? "Codex" : "Claude"}</Tag>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={hit.project_path}>
                {hit.project_path}
              </span>
              {hit.timestamp_ms && (
                <span>{new Date(hit.timestamp_ms).toLocaleString()}</span>
              )}
            </header>
            <p
              style={{
                margin: 0,
                whiteSpace: "pre-wrap",
                fontSize: "var(--fs-sm)",
              }}
            >
              {hit.snippet}
            </p>
            <footer style={{ marginTop: "var(--sp-8)", display: "flex", gap: "var(--sp-8)" }}>
              <Button
                variant="ghost"
                glyph={expanded === hit.exchange_id ? NF.chevronU : NF.chevronD}
                onClick={() => void expand(hit)}
              >
                {expanded === hit.exchange_id ? "Hide" : "Read excerpt"}
              </Button>
            </footer>
            {expanded === hit.exchange_id && (
              <pre
                style={{
                  marginTop: "var(--sp-8)",
                  padding: "var(--sp-12)",
                  background: "var(--bg-sunken)",
                  borderRadius: "var(--r-2)",
                  maxHeight: "var(--list-max-height-md)",
                  overflow: "auto",
                  whiteSpace: "pre-wrap",
                  fontSize: "var(--fs-2xs)",
                }}
              >
                {body ?? "loading..."}
              </pre>
            )}
          </article>
        ))}
      </div>

      {hasMore && (
        <SectionLabel>More results available — refine your query.</SectionLabel>
      )}
    </div>
  );
}

// ─── Memories tab ──────────────────────────────────────────────

function MemoriesTab() {
  const [rows, setRows] = useState<Memory[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [includeArchived, setIncludeArchived] = useState(false);
  const [scopeFilter, setScopeFilter] = useState<"all" | MemoryScope>("all");
  const [kindFilter, setKindFilter] = useState<"all" | MemoryKind>("all");
  const [showCreate, setShowCreate] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const list = await sharedMemoryApi.listMemories({
        scope: scopeFilter === "all" ? null : scopeFilter,
        kind: kindFilter === "all" ? null : kindFilter,
        include_archived: includeArchived,
        limit: 200,
      });
      setRows(list);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [scopeFilter, kindFilter, includeArchived]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const archive = useCallback(
    async (id: string) => {
      try {
        await sharedMemoryApi.archiveMemory(id);
        await reload();
      } catch (e) {
        setErr(String(e));
      }
    },
    [reload],
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
      <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
        <select
          value={scopeFilter}
          onChange={(e) => setScopeFilter(e.currentTarget.value as typeof scopeFilter)}
          aria-label="Scope filter"
          style={selectStyle()}
        >
          <option value="all">All scopes</option>
          <option value="global">Global</option>
          <option value="project">Project</option>
        </select>
        <select
          value={kindFilter}
          onChange={(e) => setKindFilter(e.currentTarget.value as typeof kindFilter)}
          aria-label="Kind filter"
          style={selectStyle()}
        >
          <option value="all">All kinds</option>
          <option value="fact">Fact</option>
          <option value="preference">Preference</option>
          <option value="pattern">Pattern</option>
          <option value="constraint">Constraint</option>
          <option value="summary">Summary</option>
        </select>
        <label style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)", fontSize: "var(--fs-sm)" }}>
          <input
            type="checkbox"
            checked={includeArchived}
            onChange={(e) => setIncludeArchived(e.currentTarget.checked)}
          />
          include archived
        </label>
        <div style={{ flex: 1 }} />
        {/* Demoted to ghost while the create form is open so its
            "Save" stays the view's single solid primary action. */}
        <Button
          variant={showCreate ? "ghost" : "solid"}
          glyph={NF.plus}
          onClick={() => setShowCreate((v) => !v)}
        >
          {showCreate ? "Cancel" : "Add memory"}
        </Button>
      </div>

      {showCreate && (
        <CreateMemoryForm
          onCreated={() => {
            setShowCreate(false);
            void reload();
          }}
          onCancel={() => setShowCreate(false)}
        />
      )}

      {err && <div style={{ color: "var(--danger)" }}>{err}</div>}
      {!loading && rows.length === 0 && !err && (
        <SectionLabel>No memories yet.</SectionLabel>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        {rows.map((m) => (
          <article
            key={m.id}
            style={{
              border: "var(--sp-px) solid var(--line)",
              borderRadius: "var(--r-3)",
              padding: "var(--sp-12)",
              background: "var(--bg-raised)",
              opacity: m.archived_at_ms ? 0.55 : 1,
            }}
          >
            <header style={{ display: "flex", gap: "var(--sp-8)", marginBottom: "var(--sp-6)", alignItems: "center" }}>
              <Tag>{m.kind}</Tag>
              <Tag>{m.scope}</Tag>
              {m.project_path && (
                <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                  {m.project_path}
                </span>
              )}
              <div style={{ flex: 1 }} />
              <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                {m.created_by} · {new Date(m.created_at_ms).toLocaleDateString()}
              </span>
            </header>
            <p style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: "var(--fs-sm)" }}>
              {m.content}
            </p>
            {!m.archived_at_ms && (
              <footer style={{ marginTop: "var(--sp-8)" }}>
                <Button variant="ghost" glyph={NF.archive} onClick={() => void archive(m.id)}>
                  Archive
                </Button>
              </footer>
            )}
          </article>
        ))}
      </div>
    </div>
  );
}

function CreateMemoryForm({
  onCreated,
  onCancel,
}: {
  onCreated: () => void;
  onCancel: () => void;
}) {
  const [scope, setScope] = useState<MemoryScope>("global");
  const [projectPath, setProjectPath] = useState("");
  const [kind, setKind] = useState<MemoryKind>("fact");
  const [content, setContent] = useState("");
  const [createdBy, setCreatedBy] = useState("user:me");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = useCallback(async () => {
    if (!content.trim() || !createdBy.trim()) return;
    if (scope === "project" && !projectPath.trim()) {
      setErr("project_path is required for scope=project");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await sharedMemoryApi.createMemory({
        scope,
        project_path: scope === "project" ? projectPath.trim() : null,
        kind,
        content: content.trim(),
        created_by: createdBy.trim(),
      });
      setContent("");
      onCreated();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }, [scope, projectPath, kind, content, createdBy, onCreated]);

  return (
    <div
      style={{
        border: "var(--sp-px) solid var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-16)",
        background: "var(--bg-raised)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-10)",
      }}
    >
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <select
          value={scope}
          onChange={(e) => setScope(e.currentTarget.value as MemoryScope)}
          style={selectStyle()}
        >
          <option value="global">Global</option>
          <option value="project">Project</option>
        </select>
        <select
          value={kind}
          onChange={(e) => setKind(e.currentTarget.value as MemoryKind)}
          style={selectStyle()}
        >
          <option value="fact">Fact</option>
          <option value="preference">Preference</option>
          <option value="pattern">Pattern</option>
          <option value="constraint">Constraint</option>
          <option value="summary">Summary</option>
        </select>
        {scope === "project" && (
          <Input
            value={projectPath}
            onChange={(e) => setProjectPath(e.currentTarget.value)}
            placeholder="project_path (absolute)"
            style={{ flex: 1 }}
          />
        )}
      </div>
      <textarea
        value={content}
        onChange={(e) => setContent(e.currentTarget.value)}
        placeholder="What should we remember?"
        rows={3}
        style={{
          padding: "var(--sp-8)",
          background: "var(--bg-sunken)",
          color: "var(--fg)",
          border: "var(--sp-px) solid var(--line)",
          borderRadius: "var(--r-2)",
          font: "inherit",
          resize: "vertical",
        }}
      />
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <Input
          value={createdBy}
          onChange={(e) => setCreatedBy(e.currentTarget.value)}
          placeholder="created_by (e.g. user:me)"
          style={{ flex: 1 }}
        />
        <Button onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <Button variant="solid" onClick={() => void submit()} disabled={busy || !content.trim()}>
          {busy ? "Saving…" : "Save"}
        </Button>
      </div>
      {err && <div style={{ color: "var(--danger)" }}>{err}</div>}
    </div>
  );
}

// ─── Decisions tab ─────────────────────────────────────────────

function DecisionsTab() {
  const [rows, setRows] = useState<Decision[]>([]);
  const [statusFilter, setStatusFilter] = useState<"all" | DecisionStatus>("active");
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const list = await sharedMemoryApi.listDecisions({
        status: statusFilter === "all" ? null : statusFilter,
        limit: 200,
      });
      setRows(list);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [statusFilter]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const archive = useCallback(
    async (id: string) => {
      try {
        await sharedMemoryApi.archiveDecision(id);
        await reload();
      } catch (e) {
        setErr(String(e));
      }
    },
    [reload],
  );

  const byProject = useMemo(() => {
    const m = new Map<string, Decision[]>();
    for (const d of rows) {
      const key = d.project_path ?? "(global)";
      const arr = m.get(key) ?? [];
      arr.push(d);
      m.set(key, arr);
    }
    return Array.from(m.entries());
  }, [rows]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
      <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.currentTarget.value as typeof statusFilter)}
          style={selectStyle()}
          aria-label="Status filter"
        >
          <option value="all">All statuses</option>
          <option value="active">Active</option>
          <option value="superseded">Superseded</option>
          <option value="archived">Archived</option>
        </select>
        <div style={{ flex: 1 }} />
      </div>

      {err && <div style={{ color: "var(--danger)" }}>{err}</div>}
      {!loading && rows.length === 0 && !err && (
        <SectionLabel>No decisions yet.</SectionLabel>
      )}

      {byProject.map(([proj, items]) => (
        <section key={proj}>
          <SectionLabel>{proj}</SectionLabel>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)", marginTop: "var(--sp-6)" }}>
            {items.map((d) => (
              <article
                key={d.id}
                style={{
                  border: "var(--sp-px) solid var(--line)",
                  borderRadius: "var(--r-3)",
                  padding: "var(--sp-12)",
                  background: "var(--bg-raised)",
                  opacity: d.status === "active" ? 1 : 0.6,
                }}
              >
                <header style={{ display: "flex", gap: "var(--sp-8)", marginBottom: "var(--sp-6)", alignItems: "center" }}>
                  <Tag>{d.status}</Tag>
                  {d.topic && <Tag>{d.topic}</Tag>}
                  <div style={{ flex: 1 }} />
                  <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                    {d.created_by} · {new Date(d.created_at_ms).toLocaleDateString()}
                  </span>
                </header>
                <p style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: "var(--fs-sm)", fontWeight: 500 }}>
                  {d.decision}
                </p>
                {d.rationale && (
                  <p style={{ margin: "var(--sp-6) 0 0", whiteSpace: "pre-wrap", fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
                    {d.rationale}
                  </p>
                )}
                {d.status === "active" && (
                  <footer style={{ marginTop: "var(--sp-8)" }}>
                    <Button variant="ghost" glyph={NF.archive} onClick={() => void archive(d.id)}>
                      Archive
                    </Button>
                  </footer>
                )}
              </article>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

// ─── shared utilities ──────────────────────────────────────────

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

// Re-export Glyph so the bundle keeps it available for the JSX tags
// above (some builders tree-shake otherwise).
void Glyph;
