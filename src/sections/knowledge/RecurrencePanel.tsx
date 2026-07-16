// Pending recurrences, surfaced at the top of Review.
//
// A recurrence is the failure the whole compiler exists to prevent: the
// agent hit a wall we had already learned about. The distiller detects it
// at ingest; the human decides here. Confirming is not busywork — it turns
// a soft signal into a real datum AND into an action: compile the class to
// a guard so it cannot recur, driving the dashboard's Recurrence number to
// zero.
//
// Renders nothing when there are no pending candidates (render-if-nonzero)
// so it never adds chrome to a quiet queue.

import { useCallback, useEffect, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type { RecurrenceEvent } from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { Tag } from "../../components/primitives/Tag";
import { CopyButton } from "../../components/CopyButton";
import { basename } from "../../lib/paths";
import { toUserError } from "../../lib/errors";

export function RecurrencePanel({
  onOpenMemory,
}: {
  /** Deep-link the matched lesson into the Know view. */
  onOpenMemory?: (projectPath: string, memoryId: string) => void;
}) {
  const [rows, setRows] = useState<RecurrenceEvent[]>([]);
  // `err` = a real, unexpected failure (red; the row stays). `note` = a
  // benign "already handled elsewhere" reconcile (neutral; the row leaves).
  // Conflating them was the bug: a red alarm fired while the row vanished as
  // if the action had succeeded.
  const [err, setErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    // Clear any prior banner/note first, so a successful retry doesn't leave a
    // stale red alert over a healthy queue — and so the empty-panel `return
    // null` below isn't blocked by an error that no longer holds.
    setErr(null);
    setNote(null);
    try {
      setRows(await sharedMemoryApi.recurrenceList());
    } catch (e) {
      setErr(toUserError(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = useCallback(
    async (row: RecurrenceEvent, verdict: "confirm" | "dismiss") => {
      setBusyId(row.id);
      setErr(null);
      setNote(null);
      try {
        const ok =
          verdict === "confirm"
            ? await sharedMemoryApi.recurrenceConfirm(row.id)
            : await sharedMemoryApi.recurrenceDismiss(row.id);
        // Whether it took now or was already resolved elsewhere, the backend
        // no longer has it pending — drop the row. Only the second case
        // leaves a neutral note; neither is an error.
        if (!ok) setNote("That recurrence was already handled elsewhere.");
        setRows((prev) => prev.filter((r) => r.id !== row.id));
      } catch (e) {
        // A true failure — the row stays so the user can retry.
        setErr(toUserError(e));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  // Once nothing is pending, the panel disappears entirely (a lingering note
  // never strands an empty, un-dismissable banner). A real error keeps the
  // panel so the failed row and its reason remain visible.
  if (rows.length === 0 && !err) return null;

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
      <SectionLabel>Recurrences — you learned this, and it happened again</SectionLabel>
      {err && (
        <div role="alert" style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)", flexWrap: "wrap" }}>
          <span style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>{err}</span>
          <Button variant="ghost" onClick={() => void refresh()}>
            Retry
          </Button>
        </div>
      )}
      {note && (
        <div style={{ color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>{note}</div>
      )}
      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        {rows.map((row) => (
          <li key={row.id}>
            <div
              style={{
                border: "var(--sp-px) solid color-mix(in oklch, var(--warn) 40%, var(--line))",
                borderRadius: "var(--r-3)",
                padding: "var(--sp-12) var(--sp-16)",
                background: "var(--bg-raised)",
                display: "flex",
                flexDirection: "column",
                gap: "var(--sp-6)",
              }}
            >
              <header style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center", flexWrap: "wrap" }}>
                <Tag tone="warn">recurrence</Tag>
                {row.matched_state && <Tag>{row.matched_state}</Tag>}
                <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                  matched by {row.detected_by}
                </span>
                <div style={{ flex: 1 }} />
                {row.new_file_path && (
                  <span
                    style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}
                    title={row.new_file_path}
                  >
                    seen again in {basename(row.new_file_path)}
                  </span>
                )}
              </header>

              <p style={{ margin: 0, fontWeight: 500, fontSize: "var(--fs-base)" }}>
                {row.new_content}
              </p>
              {row.matched_content && (
                <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
                  already learned: {row.matched_content}
                </p>
              )}

              <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center", flexWrap: "wrap", marginTop: "var(--sp-4)" }}>
                <Button
                  variant="solid"
                  disabled={busyId === row.id}
                  onClick={() => void act(row, "confirm")}
                >
                  {busyId === row.id ? "…" : "Confirm — still a risk"}
                </Button>
                <Button
                  variant="ghost"
                  disabled={busyId === row.id}
                  onClick={() => void act(row, "dismiss")}
                >
                  Dismiss
                </Button>
                {onOpenMemory && row.matched_memory_id && (
                  <Button
                    variant="ghost"
                    disabled={busyId === row.id}
                    onClick={() => onOpenMemory(row.project_path, row.matched_memory_id)}
                  >
                    Open in Know
                  </Button>
                )}
              </div>

              {/* Loop closure: confirming exists to then make the class
                  impossible. An accepted lesson can be compiled to a guard
                  now; a suspect one must be re-reviewed first — never enforce
                  knowledge whose code has already moved. Guard the id: a
                  malformed row must not hand over `compile null --write`. */}
              {row.matched_state === "accepted" && row.matched_memory_id ? (
                <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)", flexWrap: "wrap", fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                  <span>Stop it recurring — enforce the lesson as a guard:</span>
                  <code style={{ fontSize: "var(--fs-2xs)" }}>
                    claudepot lesson compile … --write
                  </code>
                  <CopyButton
                    text={`claudepot lesson compile ${row.matched_memory_id} --write`}
                    ariaLabel="Copy compile command"
                  />
                </div>
              ) : row.matched_state === "suspect" ? (
                <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
                  The matched lesson is suspect — re-review it in the queue below
                  before it can be enforced.
                </span>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
