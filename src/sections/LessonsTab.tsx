// The knowledge compiler's triage inbox + the honest gazette.
//
// The Memory tab used to be a library — walls of stored rows nobody
// read. This is an INBOX instead. The distiller proposes lessons; the
// user's entire job here is a yes or a no, from the keyboard. Authoring
// is what kills every personal-knowledge tool, so the user never
// authors — only judges.
//
// The dashboard at the top is deliberately NOT "N memories stored".
// That is a vanity metric. It shows the numbers that matter: how many
// are ENFORCED by a real check, how many are merely documented, and how
// many have gone SUSPECT because the code they relied on changed.

import { useCallback, useEffect, useMemo, useState } from "react";
import { sharedMemoryApi } from "../api/sharedMemory";
import type {
  LessonCounts,
  LessonRow,
  ReviewStateName,
} from "../api/sharedMemory";
import { Button } from "../components/primitives/Button";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { Tag } from "../components/primitives/Tag";
import { RecurrencePanel } from "./knowledge/RecurrencePanel";
import { StatCard } from "./knowledge/dashboard-primitives";
import type { StatCardProps } from "./knowledge/dashboard-primitives";

type QueueState = Extract<ReviewStateName, "proposed" | "suspect">;

export function LessonsTab({
  onOpenMemory,
}: {
  /** Deep-link a recurrence's matched lesson into Know. */
  onOpenMemory?: (projectPath: string, memoryId: string) => void;
}) {
  const [counts, setCounts] = useState<LessonCounts | null>(null);
  const [rows, setRows] = useState<LessonRow[]>([]);
  const [queue, setQueue] = useState<QueueState>("proposed");
  const [cursor, setCursor] = useState(0);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const [c, r] = await Promise.all([
        sharedMemoryApi.lessonCounts(),
        sharedMemoryApi.lessonList({ state: queue, limit: 200 }),
      ]);
      setCounts(c);
      setRows(r);
      setCursor((i) => Math.min(i, Math.max(0, r.length - 1)));
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [queue]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = useCallback(
    async (row: LessonRow, verdict: "accept" | "reject") => {
      setBusyId(row.id);
      try {
        // The backend resolves the lesson's project HEAD and anchors the
        // acceptance, so stale-code invalidation works from the GUI too.
        const ok =
          verdict === "accept"
            ? await sharedMemoryApi.lessonAccept({ id: row.id })
            : await sharedMemoryApi.lessonReject(row.id);
        // A `false` return means the row wasn't in the expected state
        // (already judged elsewhere, archived, stale id). Don't silently
        // drop it as if the verdict took — surface it and refresh.
        if (!ok) {
          setErr(
            `Could not ${verdict} this lesson — it may have changed. Refreshing.`,
          );
          await refresh();
          return;
        }
        // Drop the judged row from view; keep the cursor where it was so
        // the next card slides under it — the whole point of a fast queue.
        setRows((prev) => {
          const next = prev.filter((r) => r.id !== row.id);
          setCursor((i) => Math.min(i, Math.max(0, next.length - 1)));
          return next;
        });
        void sharedMemoryApi.lessonCounts().then(setCounts).catch(() => {});
      } catch (e) {
        setErr(String(e));
      } finally {
        setBusyId(null);
      }
    },
    [refresh],
  );

  // Keyboard triage: j/k move, a accept, r reject. Never while an input
  // is focused (there are none here, but the guard keeps it future-safe).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const row = rows[cursor];
      // Math.max(0, …) floors the cursor at 0 (not -1) when the queue is
      // empty — otherwise `j` on an empty queue leaves cursor at -1, which
      // the refresh/accept clamps preserve, so the first loaded card gets
      // no focus and a/r silently no-op until j/k is pressed again.
      if (e.key === "j")
        setCursor((i) => Math.min(i + 1, Math.max(0, rows.length - 1)));
      else if (e.key === "k") setCursor((i) => Math.max(i - 1, 0));
      else if (e.key === "a" && row) void act(row, "accept");
      else if (e.key === "r" && row) void act(row, "reject");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rows, cursor, act]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)" }}>
      <Gazette counts={counts} />

      {/* High-signal: a class we already learned that recurred anyway.
          Renders nothing when there are no pending candidates. */}
      <RecurrencePanel onOpenMemory={onOpenMemory} />

      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-12)" }}>
        <SectionLabel>Review queue</SectionLabel>
        <QueueToggle value={queue} counts={counts} onChange={setQueue} />
        <div style={{ flex: 1 }} />
        <Button variant="ghost" onClick={() => void refresh()} disabled={loading}>
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>

      {err && (
        <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--fs-base)" }}>
          {err}
        </div>
      )}

      {!loading && rows.length === 0 && (
        <EmptyQueue queue={queue} />
      )}

      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-12)" }}>
        {rows.map((row, i) => (
          <LessonCard
            key={row.id}
            row={row}
            focused={i === cursor}
            busy={busyId === row.id}
            onFocus={() => setCursor(i)}
            onAccept={() => void act(row, "accept")}
            onReject={() => void act(row, "reject")}
          />
        ))}
      </ul>

      {rows.length > 0 && (
        <p style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)", margin: 0 }}>
          <kbd>j</kbd>/<kbd>k</kbd> move · <kbd>a</kbd> accept · <kbd>r</kbd> reject.
          You judge; you never author.
        </p>
      )}
    </div>
  );
}

// ─── the honest gazette (Phase 5) ────────────────────────────────

function Gazette({ counts }: { counts: LessonCounts | null }) {
  // Uses the shared StatCard + toneColor from dashboard-primitives so the
  // Review gazette and the Dashboard speak one visual language (the doc's
  // "extract Gazette's StatCard + toneColor so both surfaces agree").
  const stats: StatCardProps[] = useMemo(
    () => [
      {
        label: "To review",
        value: counts?.proposed ?? 0,
        tone: "accent",
        hint: "waiting on your yes / no",
      },
      {
        label: "Enforced",
        value: counts?.enforced ?? 0,
        tone: "good",
        hint: "compiled into a check that fails the build",
      },
      {
        label: "Documented",
        value: Math.max(0, (counts?.accepted ?? 0) - (counts?.enforced ?? 0)),
        tone: "neutral",
        hint: "accepted, not yet a check",
      },
      {
        label: "Suspect",
        value: counts?.suspect ?? 0,
        tone: "warn",
        hint: "the code they relied on changed",
      },
    ],
    [counts],
  );

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fit, minmax(9rem, 1fr))",
        gap: "var(--sp-12)",
      }}
    >
      {stats.map((s) => (
        <StatCard key={s.label} {...s} />
      ))}
    </div>
  );
}

// ─── a single card ───────────────────────────────────────────────

function LessonCard({
  row,
  focused,
  busy,
  onFocus,
  onAccept,
  onReject,
}: {
  row: LessonRow;
  focused: boolean;
  busy: boolean;
  onFocus: () => void;
  onAccept: () => void;
  onReject: () => void;
}) {
  const evidence = useMemo(() => parseAnchor(row.anchor_json), [row.anchor_json]);
  return (
    <li>
      <div
        onMouseEnter={onFocus}
        style={{
          border: `var(--sp-px) solid ${focused ? "var(--accent)" : "var(--line)"}`,
          borderRadius: "var(--r-3)",
          padding: "var(--sp-16)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-8)",
          background: focused ? "var(--accent-soft)" : "transparent",
        }}
      >
        <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "baseline" }}>
          <Tag>{row.kind}</Tag>
          {typeof row.confidence === "number" && (
            <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
              {row.confidence}%
            </span>
          )}
        </div>

        <p style={{ margin: 0, fontWeight: 500 }}>{row.content}</p>

        {row.directive && (
          <p
            style={{
              margin: 0,
              fontFamily: "var(--font-mono)",
              fontSize: "var(--fs-base)",
              color: "var(--accent)",
            }}
          >
            → {row.directive}
          </p>
        )}

        {row.suspect_reason && (
          <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--warn)" }}>
            ! {row.suspect_reason}
          </p>
        )}

        {evidence && (
          <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
            because: {evidence}
          </p>
        )}

        <div style={{ display: "flex", gap: "var(--sp-8)", marginTop: "var(--sp-4)" }}>
          <Button variant="solid" onClick={onAccept} disabled={busy}>
            {busy ? "…" : "Accept"}
          </Button>
          <Button variant="ghost" onClick={onReject} disabled={busy}>
            Reject
          </Button>
        </div>
      </div>
    </li>
  );
}

// ─── queue toggle + empty states ─────────────────────────────────

function QueueToggle({
  value,
  counts,
  onChange,
}: {
  value: QueueState;
  counts: LessonCounts | null;
  onChange: (q: QueueState) => void;
}) {
  const opts: { id: QueueState; label: string; n: number }[] = [
    { id: "proposed", label: "Proposed", n: counts?.proposed ?? 0 },
    { id: "suspect", label: "Suspect", n: counts?.suspect ?? 0 },
  ];
  return (
    <div style={{ display: "flex", gap: "var(--sp-4)" }}>
      {opts.map((o) => (
        <Button
          key={o.id}
          variant={value === o.id ? "subtle" : "ghost"}
          aria-pressed={value === o.id}
          onClick={() => onChange(o.id)}
        >
          {o.label}
          {o.n > 0 ? ` · ${o.n}` : ""}
        </Button>
      ))}
    </div>
  );
}

function EmptyQueue({ queue }: { queue: QueueState }) {
  return (
    <div
      style={{
        border: "var(--sp-px) dashed var(--line)",
        borderRadius: "var(--r-3)",
        padding: "var(--sp-24)",
        textAlign: "center",
        color: "var(--fg-muted)",
      }}
    >
      {queue === "proposed" ? (
        <>
          <p style={{ margin: 0 }}>Nothing to review.</p>
          <p style={{ margin: "var(--sp-8) 0 0", fontSize: "var(--fs-sm)" }}>
            Harvest lessons from your sessions:{" "}
            <code>claudepot lesson harvest</code>
          </p>
        </>
      ) : (
        <p style={{ margin: 0 }}>
          No suspect lessons. Nothing you accepted has gone stale.
        </p>
      )}
    </div>
  );
}

// The distiller stores evidence inside the anchor JSON so it survives an
// index rebuild alongside the files/commit. Pull it out for display.
function parseAnchor(anchorJson: string | null): string | null {
  if (!anchorJson) return null;
  try {
    const v = JSON.parse(anchorJson) as { evidence?: unknown };
    return typeof v.evidence === "string" && v.evidence.length > 0
      ? v.evidence
      : null;
  } catch {
    return null;
  }
}
