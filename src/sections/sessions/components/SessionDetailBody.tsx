import type { RefObject } from "react";
import { Button } from "../../../components/primitives/Button";
import { Glyph } from "../../../components/primitives/Glyph";
import { Input } from "../../../components/primitives/Input";
import { NF } from "../../../icons";
import { redactSecrets } from "../../../lib/redactSecrets";
import type { SessionChunk, SessionEvent } from "../../../types";
import type { MetaMatch } from "../sessionDetail.search";
import { SessionChunkView } from "../SessionChunkView";
import { SessionEventView } from "../SessionEventView";
import { EmptyState } from "./SessionDetailStates";

/**
 * Render the "your query matched on row metadata, not transcript
 * text" explanation inline in the empty-state pane.
 *
 * Shown only when the user has a non-empty query AND the transcript
 * found no match AND at least one meta field carried the query. Keeps
 * a user who navigated here from the list filter from thinking the
 * detail is broken.
 */
/**
 * Exported for `SessionDetailBody.test.tsx`, which proves that every
 * string reaching the DOM is run through `redactSecrets`. No other
 * caller should import this — it's internal to the empty-state view.
 */
export function MetaMatchNote({
  query,
  matches,
}: {
  query: string;
  matches: MetaMatch[];
}) {
  if (matches.length === 0) return null;
  // Every string that crosses into the DOM here goes through
  // `redactSecrets`. The banner displays `project_path`, `git_branch`,
  // model ids, session ids, and the user's own query — every one of
  // them is free-text the user controls and could carry an
  // `sk-ant-*` substring (a path segment, a pasted branch name, or
  // the query itself). The design rule is "credentials never
  // rendered"; this is the defense-in-depth for the one place where
  // row metadata reaches the empty-state view.
  const safeQuery = redactSecrets(query);
  return (
    <div
      role="note"
      aria-live="polite"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        maxWidth: "var(--content-cap-md)",
        padding: "var(--sp-10) var(--sp-14)",
        marginTop: "var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        background: "var(--bg-sunken)",
      }}
    >
      <span
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg)",
        }}
      >
        The term <strong>"{safeQuery}"</strong> isn't inside this transcript.
        It matched on:
      </span>
      <ul
        style={{
          listStyle: "none",
          margin: 0,
          padding: 0,
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-2)",
        }}
      >
        {matches.map((m) => {
          const safeValue = redactSecrets(m.value);
          return (
            <li
              key={m.field}
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-muted)",
                display: "flex",
                gap: "var(--sp-6)",
                alignItems: "baseline",
              }}
            >
              <span style={{ color: "var(--fg-faint)", minWidth: "6.5em" }}>
                {m.field}
              </span>
              <code
                style={{
                  fontFamily: "inherit",
                  color: "var(--fg)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={safeValue}
              >
                {safeValue}
              </code>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/**
 * Search bar + scrolling transcript pane for the session detail
 * viewer. Renders the chunked path when `visibleChunksList` is
 * present, otherwise falls back to the per-event raw stream.
 *
 * Pure presentational — every paging callback, search state, and
 * sentinel ref comes in through props. The parent owns event/chunk
 * lifecycle, search debouncing, and the IntersectionObserver hook.
 *
 * Lifted out of `SessionDetail.tsx` per the project's one-component-
 * per-file rule, and to give the parent breathing room as more
 * controls land in the header strip.
 */
export function SessionDetailBody({
  viewMode,
  events,
  visible,
  hidden,
  matchCount,
  metaMatches,
  visibleChunksList,
  chunksFiltered,
  search,
  setSearch,
  topSentinelRef,
  scrollRef,
  onLoadMoreEvents,
  onLoadMoreChunks,
  eventPage,
  chunkPage,
}: {
  viewMode: "chunks" | "raw";
  events: SessionEvent[];
  /** Filtered + windowed event list with original indices (for stable
   * keys when the search narrows). */
  visible: { e: SessionEvent; i: number }[];
  /** Number of older events not yet shown (pagination footer). */
  hidden: number;
  /** Total post-filter event count — drives the search-bar match
   * counter and the empty-state branch when the search returns
   * nothing. */
  matchCount: number;
  /** Row-level meta fields that matched the current query. Populated
   * only when the transcript has zero hits; drives the inline
   * explanation banner so the viewer doesn't look silently broken
   * when the list filter landed the user here via a project-path /
   * branch / model match. */
  metaMatches: MetaMatch[];
  /** Filtered + windowed chunk list. `null` when chunks aren't
   * available (older Tauri binary) — body falls back to raw events. */
  visibleChunksList: SessionChunk[] | null;
  chunksFiltered: SessionChunk[] | null;
  search: string;
  setSearch: (q: string) => void;
  /** Sentinel that the parent's `useReachTop` observes to auto-page
   * older chunks. */
  topSentinelRef: RefObject<HTMLDivElement | null>;
  /** Callback ref the parent attaches to observe transcript scroll —
   * powers the auto-compact session header. Optional so older callers
   * don't have to wire it. */
  scrollRef?: (el: HTMLDivElement | null) => void;
  onLoadMoreEvents: () => void;
  onLoadMoreChunks: () => void;
  /** How many entries the "Show older …" buttons reveal at a time.
   * Passed in (not a constant in this file) so the parent owns the
   * pagination policy. */
  eventPage: number;
  chunkPage: number;
}) {
  const trimmedQuery = search.trim();
  return (
    <>
      {/* Search bar -------------------------------------------------- */}
      <div
        style={{
          padding: "var(--sp-10) var(--sp-28)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          flexShrink: 0,
        }}
      >
        <Input
          glyph={NF.search}
          placeholder="Search within transcript"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          style={{ flex: 1 }}
          aria-label="Search within transcript"
        />
        {search.trim().length >= 2 && (
          <span
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              whiteSpace: "nowrap",
            }}
          >
            {matchCount} match{matchCount === 1 ? "" : "es"}
          </span>
        )}
      </div>

      {/* Transcript -------------------------------------------------- */}
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-18) var(--sp-28)",
        }}
      >
        {viewMode === "chunks" && visibleChunksList ? (
          visibleChunksList.length === 0 ? (
            <EmptyState>
              <Glyph g={NF.chatAlt} color="var(--fg-ghost)" />
              {trimmedQuery
                ? "Nothing matches that query."
                : "This session has no events yet."}
              {trimmedQuery && metaMatches.length > 0 && (
                <MetaMatchNote query={trimmedQuery} matches={metaMatches} />
              )}
            </EmptyState>
          ) : (
            <>
              <div
                ref={topSentinelRef}
                data-testid="chunks-top-sentinel"
                aria-hidden
                style={{ height: 1 }}
              />
              {chunksFiltered &&
                chunksFiltered.length > visibleChunksList.length && (
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "center",
                      marginBottom: "var(--sp-14)",
                    }}
                  >
                    <Button variant="ghost" onClick={onLoadMoreChunks}>
                      Show{" "}
                      {Math.min(
                        chunksFiltered.length - visibleChunksList.length,
                        chunkPage,
                      )}{" "}
                      older chunk
                      {chunksFiltered.length - visibleChunksList.length === 1
                        ? ""
                        : "s"}
                    </Button>
                  </div>
                )}
              {visibleChunksList.map((c) => (
                <SessionChunkView
                  key={c.id}
                  chunk={c}
                  events={events}
                  searchTerm={search.trim()}
                />
              ))}
            </>
          )
        ) : matchCount === 0 ? (
          <EmptyState>
            <Glyph g={NF.chatAlt} color="var(--fg-ghost)" />
            {trimmedQuery
              ? "Nothing matches that query."
              : "This session has no events yet."}
            {trimmedQuery && metaMatches.length > 0 && (
              <MetaMatchNote query={trimmedQuery} matches={metaMatches} />
            )}
          </EmptyState>
        ) : (
          <>
            {hidden > 0 && (
              <div
                style={{
                  display: "flex",
                  justifyContent: "center",
                  marginBottom: "var(--sp-14)",
                }}
              >
                <Button variant="ghost" onClick={onLoadMoreEvents}>
                  Show {Math.min(hidden, eventPage)} older event
                  {hidden === 1 ? "" : "s"}
                </Button>
              </div>
            )}
            {visible.map(({ e, i }) => (
              <SessionEventView
                key={i}
                event={e}
                searchTerm={search.trim()}
              />
            ))}
          </>
        )}
      </div>
    </>
  );
}
