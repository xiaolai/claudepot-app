// One card in the Know view's unified knowledge stream.
//
// The Know view collapses three durable record types — memory, decision,
// evidence — into one project-grouped stream. Each renders with the same
// grammar: a kind badge + a state badge, the claim/decision/summary text,
// a provenance row ("learned from …", "enforced by …"), lazily-loaded
// cross-links, and the minimal human-gated actions (archive, re-review a
// suspect item, copy a path).

import { useCallback, useEffect, useState } from "react";
import { sharedMemoryApi } from "../../api/sharedMemory";
import type {
  Decision,
  Evidence,
  LessonRow,
  MemoryLink,
} from "../../api/sharedMemory";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { TagTone } from "../../components/primitives/Tag";
import { CopyButton } from "../../components/CopyButton";
import { basename } from "../../lib/paths";

// ─── the unified item ────────────────────────────────────────────

export type KnowItem =
  | { type: "memory"; id: string; projectPath: string | null; createdAtMs: number; row: LessonRow }
  | { type: "decision"; id: string; projectPath: string | null; createdAtMs: number; row: Decision }
  | { type: "evidence"; id: string; projectPath: string | null; createdAtMs: number; row: Evidence };

export interface StateBadge {
  label: string;
  tone: TagTone;
}

/** `true` for an accepted memory that was compiled into a guard — the
 *  "enforced" state, distinct from merely "accepted". */
export function isEnforced(row: LessonRow): boolean {
  return row.review_state === "accepted" && row.compile_target === "guard";
}

export function memoryStateBadge(row: LessonRow): StateBadge {
  switch (row.review_state) {
    case "proposed":
      return { label: "proposed", tone: "accent" };
    case "accepted":
      return isEnforced(row)
        ? { label: "enforced", tone: "ok" }
        : { label: "accepted", tone: "neutral" };
    case "suspect":
      return { label: "suspect", tone: "warn" };
    case "rejected":
      return { label: "rejected", tone: "danger" };
    default:
      return { label: row.review_state, tone: "neutral" };
  }
}

export function decisionStateBadge(row: Decision): StateBadge {
  switch (row.status) {
    case "active":
      return { label: "active", tone: "ok" };
    case "superseded":
      return { label: "superseded", tone: "ghost" };
    default:
      return { label: "archived", tone: "ghost" };
  }
}

/** The distiller stores its evidence sentence inside the anchor JSON so
 *  it survives an index rebuild. Pull it out for display. */
function parseAnchorEvidence(anchorJson: string | null): string | null {
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

function parseFilesChanged(json: string): string[] {
  try {
    const v = JSON.parse(json) as unknown;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

// ─── the card ────────────────────────────────────────────────────

export function KnowItemCard({
  item,
  focused = false,
  provenanceOpen = false,
  onToggleProvenance,
  cardRef,
  onArchived,
  onReview,
}: {
  item: KnowItem;
  /** Keyboard cursor is on this card (j/k). Shows a focus ring. */
  focused?: boolean;
  /** Whether this card's provenance excerpt is open (controlled by the
   *  view so Enter can open the focused card's). */
  provenanceOpen?: boolean;
  onToggleProvenance?: () => void;
  cardRef?: (el: HTMLDivElement | null) => void;
  /** Refetch after a successful archive. */
  onArchived: () => void;
  /** Route a suspect item to the Review tab (the queue that re-judges it). */
  onReview: () => void;
}) {
  return (
    <li>
      <div
        ref={cardRef}
        style={{
          border: `var(--sp-px) solid ${focused ? "var(--accent)" : "var(--line)"}`,
          borderRadius: "var(--r-3)",
          padding: "var(--sp-12) var(--sp-16)",
          background: focused ? "var(--accent-soft)" : "var(--bg-raised)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-8)",
        }}
      >
        {item.type === "memory" && (
          <MemoryBody
            row={item.row}
            id={item.id}
            provenanceOpen={provenanceOpen}
            onToggleProvenance={onToggleProvenance}
            onArchived={onArchived}
            onReview={onReview}
          />
        )}
        {item.type === "decision" && (
          <DecisionBody row={item.row} id={item.id} onArchived={onArchived} />
        )}
        {item.type === "evidence" && <EvidenceBody row={item.row} />}
      </div>
    </li>
  );
}

function CardHeader({
  kindLabel,
  kindTone = "neutral",
  state,
  meta,
}: {
  kindLabel: string;
  kindTone?: TagTone;
  state?: StateBadge;
  meta: string;
}) {
  return (
    <header style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center", flexWrap: "wrap" }}>
      <Tag tone={kindTone}>{kindLabel}</Tag>
      {state && <Tag tone={state.tone}>{state.label}</Tag>}
      <div style={{ flex: 1 }} />
      <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>{meta}</span>
    </header>
  );
}

// ─── memory ──────────────────────────────────────────────────────

function MemoryBody({
  row,
  id,
  provenanceOpen,
  onToggleProvenance,
  onArchived,
  onReview,
}: {
  row: LessonRow;
  id: string;
  provenanceOpen: boolean;
  onToggleProvenance?: () => void;
  onArchived: () => void;
  onReview: () => void;
}) {
  const state = memoryStateBadge(row);
  const evidence = parseAnchorEvidence(row.anchor_json);
  return (
    <>
      <CardHeader
        kindLabel={row.kind}
        state={state}
        meta={
          typeof row.confidence === "number"
            ? `${row.confidence}% · ${new Date(row.created_at_ms).toLocaleDateString()}`
            : new Date(row.created_at_ms).toLocaleDateString()
        }
      />
      <p style={{ margin: 0, fontWeight: 500, fontSize: "var(--fs-base)" }}>{row.content}</p>

      {row.directive && (
        <p
          style={{
            margin: 0,
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-sm)",
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

      {/* Provenance: the guard it compiled to, then the transcript it was
          learned from (an inline excerpt via read_locator). */}
      {isEnforced(row) && row.guard_ref && (
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)", fontSize: "var(--fs-sm)" }}>
          <span style={{ color: "var(--ok)" }}>enforced by</span>
          <code style={{ fontSize: "var(--fs-sm)" }} title={row.guard_ref}>
            {row.guard_ref}
          </code>
          <CopyButton text={row.guard_ref} ariaLabel={`Copy guard path ${row.guard_ref}`} />
        </div>
      )}

      {row.origin_file_path && (
        <Provenance
          filePath={row.origin_file_path}
          exchangeId={row.origin_exchange_id}
          open={provenanceOpen}
          onToggle={onToggleProvenance}
        />
      )}

      <CrossLinks memoryId={id} />

      <footer style={{ display: "flex", gap: "var(--sp-8)", marginTop: "var(--sp-4)" }}>
        {row.review_state === "suspect" && (
          <Button variant="subtle" onClick={onReview}>
            Re-review
          </Button>
        )}
        <ArchiveButton
          label="Archive"
          onArchive={() => sharedMemoryApi.archiveMemory(id)}
          onDone={onArchived}
        />
      </footer>
    </>
  );
}

// ─── decision ────────────────────────────────────────────────────

function DecisionBody({
  row,
  id,
  onArchived,
}: {
  row: Decision;
  id: string;
  onArchived: () => void;
}) {
  return (
    <>
      <CardHeader
        kindLabel={row.topic ? `decision · ${row.topic}` : "decision"}
        kindTone="accent"
        state={decisionStateBadge(row)}
        meta={`${row.created_by} · ${new Date(row.created_at_ms).toLocaleDateString()}`}
      />
      <p style={{ margin: 0, fontWeight: 500, fontSize: "var(--fs-base)" }}>{row.decision}</p>
      {row.rationale && (
        <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
          {row.rationale}
        </p>
      )}
      {row.supersedes_id && (
        <p style={{ margin: 0, fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
          supersedes an earlier decision
        </p>
      )}

      <CrossLinks decisionId={id} />

      {row.status === "active" && (
        <footer style={{ marginTop: "var(--sp-4)" }}>
          <ArchiveButton
            label="Archive"
            onArchive={() => sharedMemoryApi.archiveDecision(id)}
            onDone={onArchived}
          />
        </footer>
      )}
    </>
  );
}

// ─── evidence ────────────────────────────────────────────────────

function EvidenceBody({ row }: { row: Evidence }) {
  const files = parseFilesChanged(row.files_changed_json);
  return (
    <>
      <CardHeader
        kindLabel={row.topic ? `evidence · ${row.topic}` : "evidence"}
        state={{ label: `${row.confidence}%`, tone: "neutral" }}
        meta={`${row.created_by} · ${new Date(row.created_at_ms).toLocaleDateString()}`}
      />
      <p style={{ margin: 0, fontWeight: 500, fontSize: "var(--fs-base)" }}>{row.summary}</p>
      <p style={{ margin: 0, fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
        verified: {row.verification}
      </p>
      {files.length > 0 && (
        <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--sp-6)" }}>
          {files.map((f) => (
            <code
              key={f}
              title={f}
              style={{
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-muted)",
                background: "var(--bg-sunken)",
                padding: "0 var(--sp-4)",
                borderRadius: "var(--r-1)",
              }}
            >
              {basename(f)}
            </code>
          ))}
        </div>
      )}
    </>
  );
}

// ─── provenance (inline excerpt) ─────────────────────────────────

function Provenance({
  filePath,
  exchangeId,
  open,
  onToggle,
}: {
  filePath: string;
  exchangeId: string | null;
  /** Controlled by the view, so Enter can open the focused card's. */
  open: boolean;
  onToggle?: () => void;
}) {
  const [body, setBody] = useState<string | null>(null);

  // Fetch the excerpt the first time it is opened. Controlled `open`
  // means this can be triggered by a click OR by the keyboard cursor.
  useEffect(() => {
    if (!open || body != null) return;
    let cancelled = false;
    void (async () => {
      try {
        const r = await sharedMemoryApi.readLocator({
          file_path: filePath,
          exchange_id: exchangeId,
          max_bytes: 8 * 1024,
        });
        if (!cancelled) setBody(r.body);
      } catch (e) {
        if (!cancelled) setBody(`error: ${e}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, body, filePath, exchangeId]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)", fontSize: "var(--fs-sm)" }}>
        <span style={{ color: "var(--fg-muted)" }}>learned from</span>
        <button
          type="button"
          className="pm-focus"
          onClick={onToggle}
          title={filePath}
          style={{
            background: "transparent",
            border: "none",
            padding: 0,
            font: "inherit",
            fontSize: "var(--fs-sm)",
            color: "var(--accent)",
            cursor: "pointer",
            textDecoration: "underline",
          }}
        >
          {basename(filePath)}
        </button>
      </div>
      {open && (
        <pre
          style={{
            margin: 0,
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
      )}
    </div>
  );
}

// ─── cross-links (lazy) ──────────────────────────────────────────

function CrossLinks({
  memoryId,
  decisionId,
  evidenceId,
}: {
  memoryId?: string;
  decisionId?: string;
  evidenceId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [links, setLinks] = useState<MemoryLink[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const toggle = useCallback(async () => {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    if (links != null) return;
    try {
      const r = await sharedMemoryApi.memoryLinks({
        memory_id: memoryId ?? null,
        decision_id: decisionId ?? null,
        evidence_id: evidenceId ?? null,
      });
      setLinks(r);
    } catch (e) {
      setErr(String(e));
    }
  }, [open, links, memoryId, decisionId, evidenceId]);

  return (
    <div>
      <button
        type="button"
        className="pm-focus"
        onClick={() => void toggle()}
        aria-expanded={open}
        style={{
          background: "transparent",
          border: "none",
          padding: 0,
          font: "inherit",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-muted)",
          cursor: "pointer",
        }}
      >
        {open ? "Hide links" : "Show links"}
      </button>
      {open && (
        <div style={{ marginTop: "var(--sp-6)", fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
          {err && <span style={{ color: "var(--danger)" }}>{err}</span>}
          {!err && links != null && links.length === 0 && <span>No links recorded.</span>}
          {!err &&
            links?.map((l) => (
              <div key={l.id}>
                {l.relation} → {l.exchange_id ?? (l.file_path ? basename(l.file_path) : "?")}
              </div>
            ))}
        </div>
      )}
    </div>
  );
}

// ─── archive (single-click, matches the app's ghost-archive pattern) ──
//
// Archive is a reversible soft-delete (sets archived_at_ms / flips a
// decision to 'archived'); the ghost variant keeps it well below the
// screen's primary action, matching the old Memories/Decisions rows. On a
// stale row (archived elsewhere) the backend returns false and we say so.

function ArchiveButton({
  label,
  onArchive,
  onDone,
}: {
  label: string;
  onArchive: () => Promise<boolean>;
  onDone: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const run = useCallback(async () => {
    setBusy(true);
    setErr(null);
    try {
      const ok = await onArchive();
      if (ok) onDone();
      else setErr("Already archived elsewhere.");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }, [onArchive, onDone]);

  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-6)" }}>
      <Button variant="ghost" onClick={() => void run()} disabled={busy}>
        {busy ? "…" : label}
      </Button>
      {err && <span style={{ fontSize: "var(--fs-2xs)", color: "var(--danger)" }}>{err}</span>}
    </span>
  );
}
