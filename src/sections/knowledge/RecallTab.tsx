// Recall — FTS search over raw indexed transcripts (Claude + Codex).
//
// Explicitly NOT the curated base (that is Know). This is the "where did I
// see this before" surface: full-text search across every exchange, with
// an inline excerpt reader, keyboard nav (j/k move, enter expands), and
// offset paging.

import { useCallback, useEffect, useRef, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { SearchHit } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { toExcerptError, toUserError } from "../../lib/errors";

type SourceFilter = "all" | "claude_code" | "codex";
const PAGE = 25;

export function RecallTab() {
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [body, setBody] = useState<string | null>(null);
  const [bodyErr, setBodyErr] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  // Generation tickets guard against out-of-order responses: only the newest
  // request may commit its result. `searchReq` protects the hit list (a fast
  // source-switch or search launches overlapping searches); `bodyReq`
  // protects the excerpt body (click hit A then B before A resolves).
  const searchReq = useRef(0);
  const bodyReq = useRef(0);

  const runSearch = useCallback(
    async (append: boolean, sourceOverride?: SourceFilter) => {
      const q = query.trim();
      // A disabled button still lets Enter submit the form — guard the empty
      // query here so a blank/whitespace search never hits the backend.
      if (!q) return;
      const src = sourceOverride ?? sourceFilter;
      const ticket = ++searchReq.current;
      setLoading(true);
      setErr(null);
      if (!append) {
        setExpanded(null);
        setBody(null);
        setBodyErr(null);
        setCursor(0);
      }
      const nextOffset = append ? offset : 0;
      try {
        const res = await sharedMemoryApi.search({
          query: q,
          source_kind: src === "all" ? null : src,
          limit: PAGE,
          offset: nextOffset,
        });
        // A newer search started while this awaited — drop this result so a
        // slow old-source response can't overwrite (or append onto) the new
        // hits, and can't corrupt the offset.
        if (ticket !== searchReq.current) return;
        setHits((prev) => (append ? [...prev, ...res.hits] : res.hits));
        // Don't leave a live "Load more" when an append returned nothing.
        setHasMore(append && res.hits.length === 0 ? false : res.has_more);
        setOffset(nextOffset + res.hits.length);
      } catch (e) {
        if (ticket === searchReq.current) setErr(toUserError(e));
      } finally {
        if (ticket === searchReq.current) setLoading(false);
      }
    },
    [query, sourceFilter, offset],
  );

  const loadBody = useCallback(async (hit: SearchHit) => {
    setBody(null);
    setBodyErr(null);
    const ticket = ++bodyReq.current;
    try {
      const r = await sharedMemoryApi.readLocator({
        file_path: hit.file_path,
        exchange_id: hit.exchange_id,
        max_bytes: 8 * 1024,
      });
      if (ticket === bodyReq.current) setBody(r.body);
    } catch (e) {
      if (ticket === bodyReq.current) setBodyErr(toExcerptError(e));
    }
  }, []);

  const expand = useCallback(
    async (hit: SearchHit) => {
      if (expanded === hit.exchange_id) {
        setExpanded(null);
        setBody(null);
        setBodyErr(null);
        return;
      }
      setExpanded(hit.exchange_id);
      await loadBody(hit);
    },
    [expanded, loadBody],
  );

  // Keyboard: j/k move the cursor, enter expands the focused hit. Never while
  // the query input or a select is focused.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      // Never hijack typing, a select, or a focused button's own Enter.
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || tag === "BUTTON")
        return;
      if (hits.length === 0) return;
      if (e.key === "j") setCursor((i) => Math.min(i + 1, hits.length - 1));
      else if (e.key === "k") setCursor((i) => Math.max(i - 1, 0));
      else if (e.key === "Enter") {
        const hit = hits[cursor];
        if (hit) {
          e.preventDefault();
          void expand(hit);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hits, cursor, expand]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch(false);
        }}
        style={{ display: "flex", gap: "var(--sp-8)" }}
      >
        <Input
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          placeholder='e.g. "rate limiter" or a file path'
          aria-label="Search query"
          style={{ flex: 1 }}
        />
        <select
          value={sourceFilter}
          onChange={(e) => {
            const next = e.currentTarget.value as SourceFilter;
            setSourceFilter(next);
            // Re-run with the new source so the visible hits actually reflect
            // the selection instead of staying stale until the next click.
            if (query.trim()) void runSearch(false, next);
          }}
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
          {loading ? "Searching…" : "Search"}
        </Button>
      </form>

      {err && (
        <div role="alert" style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
          <span style={{ color: "var(--danger)" }}>{err}</span>
          <Button variant="ghost" onClick={() => void runSearch(false)} disabled={loading}>
            Retry
          </Button>
        </div>
      )}

      {!loading && hits.length === 0 && !err && (
        <SectionLabel>
          {query.trim() ? "No matches." : "Enter a query to search raw transcripts."}
        </SectionLabel>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        {hits.map((hit, i) => (
          <article
            key={hit.exchange_id}
            onMouseEnter={() => setCursor(i)}
            style={{
              border: `var(--sp-px) solid ${i === cursor ? "var(--accent)" : "var(--line)"}`,
              borderRadius: "var(--r-3)",
              padding: "var(--sp-12)",
              background: i === cursor ? "var(--accent-soft)" : "var(--bg-raised)",
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
            {expanded === hit.exchange_id &&
              (bodyErr ? (
                <div
                  role="alert"
                  style={{ marginTop: "var(--sp-8)", display: "flex", alignItems: "center", gap: "var(--sp-6)", fontSize: "var(--fs-2xs)", color: "var(--danger)" }}
                >
                  <span>{bodyErr}</span>
                  <Button variant="ghost" onClick={() => void loadBody(hit)}>
                    Retry
                  </Button>
                </div>
              ) : (
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
                  {body ?? "loading…"}
                </pre>
              ))}
          </article>
        ))}
      </div>

      {hasMore && (
        <Button variant="ghost" onClick={() => void runSearch(true)} disabled={loading}>
          {loading ? "Loading…" : "Load more"}
        </Button>
      )}
    </div>
  );
}
