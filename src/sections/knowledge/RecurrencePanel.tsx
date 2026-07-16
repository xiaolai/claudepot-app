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
import { basename } from "../../lib/paths";

export function RecurrencePanel({
  onOpenMemory,
}: {
  /** Deep-link the matched lesson into the Know view. */
  onOpenMemory?: (projectPath: string, memoryId: string) => void;
}) {
  const [rows, setRows] = useState<RecurrenceEvent[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setRows(await sharedMemoryApi.recurrenceList());
    } catch (e) {
      setErr(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = useCallback(
    async (row: RecurrenceEvent, verdict: "confirm" | "dismiss") => {
      setBusyId(row.id);
      setErr(null);
      try {
        const ok =
          verdict === "confirm"
            ? await sharedMemoryApi.recurrenceConfirm(row.id)
            : await sharedMemoryApi.recurrenceDismiss(row.id);
        if (!ok) {
          setErr("This recurrence was already handled elsewhere.");
        }
        setRows((prev) => prev.filter((r) => r.id !== row.id));
      } catch (e) {
        setErr(String(e));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  if (rows.length === 0 && !err) return null;

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
      <SectionLabel>Recurrences — you learned this, and it happened again</SectionLabel>
      {err && (
        <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>
          {err}
        </div>
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

              <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center", marginTop: "var(--sp-4)" }}>
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
                {onOpenMemory && (
                  <Button
                    variant="ghost"
                    disabled={busyId === row.id}
                    onClick={() => onOpenMemory(row.project_path, row.matched_memory_id)}
                  >
                    Open in Know
                  </Button>
                )}
                <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
                  Confirming counts it toward the recurrence metric. Compile
                  the underlying lesson to a guard so it can’t recur again.
                </span>
              </div>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
