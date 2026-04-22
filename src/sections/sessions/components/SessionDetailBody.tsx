import type { RefObject } from "react";
import { Button } from "../../../components/primitives/Button";
import { Glyph } from "../../../components/primitives/Glyph";
import { Input } from "../../../components/primitives/Input";
import { NF } from "../../../icons";
import type { SessionChunk, SessionEvent } from "../../../types";
import { SessionChunkView } from "../SessionChunkView";
import { SessionEventView } from "../SessionEventView";
import { EmptyState } from "./SessionDetailStates";

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
  visibleChunksList,
  chunksFiltered,
  search,
  setSearch,
  topSentinelRef,
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
  /** Filtered + windowed chunk list. `null` when chunks aren't
   * available (older Tauri binary) — body falls back to raw events. */
  visibleChunksList: SessionChunk[] | null;
  chunksFiltered: SessionChunk[] | null;
  search: string;
  setSearch: (q: string) => void;
  /** Sentinel that the parent's `useReachTop` observes to auto-page
   * older chunks. */
  topSentinelRef: RefObject<HTMLDivElement | null>;
  onLoadMoreEvents: () => void;
  onLoadMoreChunks: () => void;
  /** How many entries the "Show older …" buttons reveal at a time.
   * Passed in (not a constant in this file) so the parent owns the
   * pagination policy. */
  eventPage: number;
  chunkPage: number;
}) {
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
              {search.trim()
                ? "Nothing matches that query."
                : "This session has no events yet."}
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
            {search.trim()
              ? "Nothing matches that query."
              : "This session has no events yet."}
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
