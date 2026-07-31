// Boards — a top-level section. Master/detail over durable
// agent-written boards.
//
// **Why top-level, not the Activities sub-tab this first shipped as.**
// Plan §3 decided `board` is a domain noun, and every other noun has a
// top-level home (account → Accounts, project → Projects, agent →
// Agents); filing a noun under another section contradicts that. The
// two surfaces are also different in kind: Activities is DERIVED and
// ephemeral — cards and rollups computed from CC's filesystem — while
// a board is AUTHORED and durable, its own store, data that exists
// nowhere else. Nesting the persistent thing inside the transient one
// inverted the relationship.
//
// The objection to a tenth tab was that `useSection` binds ⌘1..⌘9 to
// the first nine. Boards sits NINTH, before Settings, so it takes ⌘9;
// Settings drops to tenth and loses its number binding but keeps `⌘,`
// (see useShellShortcuts), which is its canonical shortcut. Nothing
// else moves.
//
// Three rules from `rules/design.md` shape this file:
//
//   - The list is scan+drill with one primary verb, so it is a TABLE,
//     not cards.
//   - ONE signal per surface. A push never raises a toast; the board
//     header carries the only live indicator. A board taking 200 pushes
//     an hour would otherwise make the app unusable.
//   - Provenance renders as a claim. The column header says "Reported
//     writer" and the value is prefixed, because `writer_id` is
//     self-declared and core cannot verify it (plan §8.5).

import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api";
import type { BoardDetail, BoardSummary } from "../../types";
import { Button } from "../../components/primitives/Button";
import { BackAffordance } from "../../components/primitives/BackAffordance";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { Table, Td, Th, Tr } from "../../components/primitives/Table";
import { SkeletonList } from "../../components/primitives/Skeleton";
import { save } from "@tauri-apps/plugin-dialog";
import { ScreenHeader } from "../../shell/ScreenHeader";
import { WidgetView } from "./WidgetView";

/** How often to ask whether another process committed. */
const POLL_MS = 4000;

/** Older than this with no writes and a board reads as stale. */
const STALE_MS = 1000 * 60 * 60 * 24 * 30;

function relative(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "—";
  const secs = Math.max(0, (Date.now() - then) / 1000);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

type BoardState = "live" | "idle" | "stale";

function boardState(updatedAt: string): BoardState {
  const age = Date.now() - new Date(updatedAt).getTime();
  if (!Number.isFinite(age)) return "idle";
  if (age > STALE_MS) return "stale";
  if (age < 1000 * 60 * 5) return "live";
  return "idle";
}

/** Text + glyph, never colour alone — `rules/design.md` a11y floor. */
function StateTag({ state }: { state: BoardState }) {
  const map = {
    live: { g: NF.play, label: "Live", color: "var(--ok)" },
    idle: { g: NF.clock, label: "Idle", color: "var(--fg-muted)" },
    stale: { g: NF.archive, label: "Stale", color: "var(--fg-faint)" },
  } as const;
  const m = map[state];
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
        color: m.color,
        fontSize: "var(--fs-xs)",
      }}
    >
      <Glyph g={m.g} />
      {m.label}
    </span>
  );
}

export function BoardsSection() {
  const [boards, setBoards] = useState<BoardSummary[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<BoardDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Separate from `error`, which only renders on the LIST screen. A
  // detail/export/delete failure used to set `error` and then never
  // show it, because the detail view has no branch that reads it.
  const [actionError, setActionError] = useState<string | null>(null);
  /** Pending updates while the user is reading. See `applyPending`. */
  const [pending, setPending] = useState(false);
  const version = useRef<number | null>(null);
  /** A user who has scrolled is reading; do not move the view. */
  const reading = useRef(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Bind the "user is reading" guard to the scroll container by its
  // explicit `data-board-scroll-root` marker, which this section now
  // owns directly.
  //
  // Two earlier versions were inert: an `onScroll` on a child div that
  // never scrolled, then an ancestor walk that ran before async content
  // arrived (nothing overflowed yet, so it picked `window`) and would
  // silently re-target if any wrapper gained a scrollbar. A named hook
  // is either found or it is not.
  useEffect(() => {
    const el = rootRef.current;
    const target =
      el?.closest<HTMLElement>("[data-board-scroll-root]") ?? null;
    if (!target) return;
    const onScroll = () => {
      reading.current = true;
    };
    target.addEventListener("scroll", onScroll, { passive: true });
    return () => target.removeEventListener("scroll", onScroll);
    // `boards` is a dependency because the FIRST render is a skeleton
    // with no ref — the effect returned early, and without this it
    // never re-ran once the data arrived, so the listener was never
    // attached and the guard was inert a second time. A test caught it.
  }, [selected, detail, boards]);

  const loadList = useCallback(async () => {
    try {
      setBoards(await api.boardList());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Which board the newest in-flight detail request is for. Without
  // this, opening A then B while A is slow lets A's response land
  // under B's selection — and Export/Delete read `detail.board_id`,
  // so the user could delete a board they are not looking at.
  const wanted = useRef<string | null>(null);

  const loadDetail = useCallback(async (id: string) => {
    wanted.current = id;
    try {
      const next = await api.boardDetail(id);
      if (wanted.current !== id) return; // superseded
      setDetail(next);
      setActionError(null);
    } catch (e) {
      if (wanted.current !== id) return;
      setActionError(String(e));
    }
  }, []);

  useEffect(() => {
    void loadList();
  }, [loadList]);

  useEffect(() => {
    // Drop the previous board's error, and its detail, before loading
    // the next one — otherwise a failure on board A renders over
    // board B while B is still in flight.
    setActionError(null);
    setDetail(null);
    if (selected) void loadDetail(selected);
  }, [selected, loadDetail]);

  // Poll `data_version` rather than subscribing: the writers are other
  // processes, so there is no in-process event to listen for.
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const v = await api.boardDataVersion();
        if (!alive) return;
        if (version.current === null) {
          version.current = v;
          return;
        }
        if (v !== version.current) {
          version.current = v;
          // If the user is mid-read, count the change instead of
          // yanking the view out from under them.
          // A boolean, not a tally: `data_version` says *something*
          // committed and carries no count, so incrementing it reported
          // poll ticks as if they were updates.
          if (reading.current) setPending(true);
          else {
            void loadList();
            if (selected) void loadDetail(selected);
          }
        }
      } catch {
        /* a transient read failure is not worth a banner */
      }
    };
    const h = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(h);
    };
  }, [selected, loadList, loadDetail]);

  const applyPending = useCallback(() => {
    setPending(false);
    reading.current = false;
    void loadList();
    if (selected) void loadDetail(selected);
  }, [selected, loadList, loadDetail]);

  if (error && !boards) {
    return (
      <div style={{ padding: "var(--sp-16)", color: "var(--danger)" }}>{error}</div>
    );
  }
  if (!boards) return <SkeletonList rows={4} />;

  if (boards.length === 0) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
        <ScreenHeader title="Boards" subtitle="Durable surfaces agents write into" />
      <div style={{ padding: "var(--sp-24)", color: "var(--fg-muted)" }}>
        <p style={{ marginTop: 0 }}>No boards yet.</p>
        <p style={{ fontSize: "var(--fs-sm)" }}>
          An agent or script creates one by writing to it. Boards are on
          trial — see <code>claudepot experimental board --help</code>.
        </p>
      </div>
      </div>
    );
  }

  // A selected board with no detail is either loading or failed. The
  // previous version fell through to the LIST here, so a failed detail
  // load silently bounced the user back with no explanation.
  if (selected && (!detail || detail.board_id !== selected)) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
        <ScreenHeader title="Boards" />
        <div style={{ padding: "var(--sp-16)" }}>
          <BackAffordance
            label="Boards"
            title="Back to the board list"
            onClick={() => {
              setSelected(null);
              setActionError(null);
            }}
          />
          {actionError ? (
            <p
              role="alert"
              style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}
            >
              {actionError}
            </p>
          ) : (
            <SkeletonList rows={3} />
          )}
        </div>
      </div>
    );
  }

  // Belt and braces: even with the request token, never render a
  // detail whose id disagrees with the selection.
  if (selected && detail && detail.board_id === selected) {
    return (
      <BoardDetailView
        rootRef={rootRef}
        actionError={actionError}
        onActionError={setActionError}
        detail={detail}
        pending={pending}
        onApplyPending={applyPending}
        onBack={() => {
          setSelected(null);
          reading.current = false;
          setPending(false);
        }}
        onDeleted={() => {
          setSelected(null);
          void loadList();
        }}
      />
    );
  }

  const totalRows = boards.reduce((n, b) => n + b.total_rows, 0);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <ScreenHeader
        title="Boards"
        subtitle={`${boards.length} board${boards.length === 1 ? "" : "s"} · ${totalRows} row${totalRows === 1 ? "" : "s"}`}
      />
      <div
        ref={rootRef}
        data-board-scroll-root=""
        style={{ padding: "var(--sp-16)", overflow: "auto", flex: 1, minHeight: 0 }}
      >
        {pending && <LiveBanner onApply={applyPending} />}
      <Table>
        <thead>
          <Tr>
            <Th>Name</Th>
            <Th>State</Th>
            <Th>Last update</Th>
            {/* Header says "Reported" because the value is a claim. */}
            <Th title="Self-declared by the writer; Claudepot does not verify it">
              Reported writer
            </Th>
            <Th align="right">Rows</Th>
          </Tr>
        </thead>
        <tbody>
          {boards.map((b) => (
            <Tr key={b.board_id}>
              {/* One focusable control per row rather than a clickable
                  <tr>: a row click is unreachable by keyboard, and the
                  a11y floor requires every interactive element to be. */}
              <Td title={b.name}>
                <button
                  type="button"
                  className="pm-focus"
                  onClick={() => setSelected(b.board_id)}
                  style={{
                    background: "none",
                    border: "none",
                    padding: 0,
                    color: "inherit",
                    font: "inherit",
                    cursor: "pointer",
                    textAlign: "left",
                  }}
                >
                  {b.name}
                </button>
              </Td>
              <Td>
                <StateTag state={boardState(b.updated_at)} />
              </Td>
              <Td title={b.updated_at}>{relative(b.updated_at)}</Td>
              <Td>
                {b.reported_writer ? (
                  <span title="Self-declared; not verified">
                    Reported by: {b.reported_writer}
                  </span>
                ) : (
                  <span style={{ color: "var(--fg-faint)" }}>—</span>
                )}
              </Td>
              {/* Render-if-nonzero: a 0 here is a real count, not a
                  placeholder, so it stays. */}
              <Td align="right">{b.total_rows}</Td>
            </Tr>
          ))}
        </tbody>
        </Table>
      </div>
    </div>
  );
}

/**
 * The ONE live signal for this surface.
 *
 * Not a toast — a board taking 200 pushes an hour would bury the app.
 * Not automatic either: the user has scrolled, so they are reading, and
 * replacing the data under them is the failure this prevents.
 */
function LiveBanner({ onApply }: { onApply: () => void }) {
  return (
    <div
      role="status"
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: "var(--sp-12)",
        padding: "var(--sp-8) var(--sp-12)",
        marginBottom: "var(--sp-8)",
        border: "var(--bw-hair) solid var(--border)",
        borderRadius: "var(--radius-sm)",
        background: "var(--bg-raised)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <span>New updates arrived while you were reading</span>
      <Button variant="subtle" onClick={onApply}>
        Jump to latest
      </Button>
    </div>
  );
}

function BoardDetailView({
  rootRef,
  actionError,
  onActionError,
  detail,
  pending,
  onApplyPending,
  onBack,
  onDeleted,
}: {
  rootRef: React.RefObject<HTMLDivElement | null>;
  actionError: string | null;
  onActionError: (e: string | null) => void;
  detail: BoardDetail;
  pending: boolean;
  onApplyPending: () => void;
  onBack: () => void;
  onDeleted: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const state = boardState(detail.updated_at);

  return (
    <div
      ref={rootRef}
      data-board-scroll-root=""
      style={{ padding: "var(--sp-16)", overflow: "auto", height: "100%" }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-12)",
          marginBottom: "var(--sp-12)",
        }}
      >
        <BackAffordance label="Boards" title="Back to the board list" onClick={onBack} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: "var(--fs-md)", fontWeight: 600 }}>
            {detail.name}
          </div>
          <div
            style={{
              display: "flex",
              gap: "var(--sp-12)",
              alignItems: "center",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
            }}
          >
            <StateTag state={state} />
            <span title={detail.updated_at}>
              updated {relative(detail.updated_at)}
            </span>
            {detail.source_board_id && <span>imported</span>}
          </div>
        </div>
        <Button
          variant="ghost"
          glyph={NF.download}
          onClick={async () => {
            // A bare filename resolved against the app's working
            // directory — wherever the OS launched it from. Let the
            // user choose; the command now refuses a relative path.
            const target = await save({
              defaultPath: `${detail.name}.board.json`,
              filters: [{ name: "Board export", extensions: ["json"] }],
            });
            if (!target) return;
            setBusy(true);
            onActionError(null);
            try {
              await api.boardExport(detail.board_id, target);
            } catch (e) {
              // A silent failure here reads as a successful export of
              // the only copy of this data.
              onActionError(`Export failed: ${e}`);
            } finally {
              setBusy(false);
            }
          }}
          disabled={busy}
        >
          Export
        </Button>
        {confirming ? (
          <>
            {/* High-stakes action, so text not icon, per rules/icon-buttons.md */}
            <Button variant="ghost" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            <Button
              variant="solid"
              danger
              onClick={async () => {
                try {
                  await api.boardRemove(detail.board_id);
                  onDeleted();
                } catch (e) {
                  // Reporting a delete that did not happen is worse
                  // than the failure itself.
                  onActionError(`Delete failed: ${e}`);
                }
              }}
            >
              Delete permanently
            </Button>
          </>
        ) : (
          <Button variant="ghost" onClick={() => setConfirming(true)}>
            Delete
          </Button>
        )}
      </div>

      {confirming && (
        // Disabled/destructive states state their reason inline, never
        // in a tooltip.
        <p
          style={{
            margin: "0 0 var(--sp-12)",
            fontSize: "var(--fs-sm)",
            color: "var(--danger)",
          }}
        >
          Boards are user data and nothing backs them up. Export first —
          this cannot be undone.
        </p>
      )}

      {actionError && (
        <p
          role="alert"
          style={{
            margin: "0 0 var(--sp-12)",
            fontSize: "var(--fs-sm)",
            color: "var(--danger)",
          }}
        >
          {actionError}
        </p>
      )}

      {pending && <LiveBanner onApply={onApplyPending} />}

      {/* The caveat comes from the backend so it cannot drift from the
          layer that knows it is true. */}
      <p
        style={{
          margin: "0 0 var(--sp-12)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        {detail.provenance_note}
      </p>

      <div
        style={{
          display: "grid",
          gap: "var(--sp-12)",
          gridTemplateColumns:
            "repeat(auto-fit, minmax(var(--board-widget-min-w), 1fr))",
        }}
      >
        {detail.widgets.map((w) => (
          <WidgetView key={w.id} widget={w} />
        ))}
      </div>

      <div style={{ marginTop: "var(--sp-16)" }}>
        <Table density="compact">
          <thead>
            <Tr>
              <Th>Series</Th>
              <Th>Columns</Th>
              <Th align="right">Rows</Th>
              <Th>Reported writer</Th>
            </Tr>
          </thead>
          <tbody>
            {detail.series.map((s) => (
              <Tr key={s.name}>
                <Td>{s.name}</Td>
                <Td style={{ color: "var(--fg-muted)" }}>
                  {s.columns.map((c) => `${c.name}:${c.ty}`).join(", ")}
                </Td>
                <Td align="right">{s.row_count}</Td>
                <Td>
                  {s.reported_writer ? (
                    <span title={s.last_pushed_at ?? undefined}>
                      Reported by: {s.reported_writer}
                    </span>
                  ) : (
                    <span style={{ color: "var(--fg-faint)" }}>—</span>
                  )}
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      </div>
    </div>
  );
}
