// Recall — FTS search over raw indexed transcripts (Claude + Codex).
//
// Explicitly NOT the curated base (that is Know). This is the "where did I
// see this before" surface: full-text search across every exchange, with
// an inline excerpt reader. Unchanged in substance from the old Search
// tab — it works; it stays.

import { useCallback, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { SearchHit } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";

export function RecallTab() {
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<"all" | "claude_code" | "codex">("all");
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
          onChange={(e) => setSourceFilter(e.currentTarget.value as typeof sourceFilter)}
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
        <Button type="submit" variant="solid" glyph={NF.search} disabled={!query.trim() || loading}>
          Search
        </Button>
      </form>

      {err && <div style={{ color: "var(--danger)" }}>{err}</div>}

      {!loading && hits.length === 0 && !err && (
        <SectionLabel>
          {query.trim() ? "No matches." : "Enter a query to search raw transcripts."}
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
              <span
                style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                title={hit.project_path}
              >
                {hit.project_path}
              </span>
              {hit.timestamp_ms && <span>{new Date(hit.timestamp_ms).toLocaleString()}</span>}
            </header>
            <p style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: "var(--fs-sm)" }}>
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
