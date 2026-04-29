import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { FilterChip } from "../../components/primitives/FilterChip";
import { Input } from "../../components/primitives/Input";
import { NF } from "../../icons";
import type { PruneFilterInput, PrunePlan } from "../../types";
import { formatSize } from "../projects/format";
import { CleanupPlanPreview } from "./components/CleanupPlanPreview";
import { SessionIndexRebuild } from "./components/SessionIndexRebuild";
import { SlimSubsection } from "./components/SlimSubsection";

/**
 * Cleanup tab inside Sessions — builds a PruneFilter, previews the
 * plan, and dispatches the async prune op. Follows paper-mono design:
 * single primary button, render-if-nonzero, chips for the booleans.
 *
 * State shape tracks the DTO 1:1 (older_than_secs / larger_than_bytes)
 * so a "save this filter" feature can one-day serialize it directly.
 */
export function CleanupPane({
  onOpChange,
  onTrashChanged,
  setToast,
}: {
  /** Called with the op_id when a prune starts. */
  onOpChange?: (opId: string | null) => void;
  /** Called after a prune is dispatched so the parent can nudge refreshes. */
  onTrashChanged?: () => void;
  /** Sessions-section toast setter, used by the SessionIndexRebuild
   *  subsection (the existing prune/slim flows surface errors
   *  inline). Optional so callers that don't need Rebuild still
   *  compile. */
  setToast?: (msg: string) => void;
}) {
  const [olderThanDays, setOlderThanDays] = useState<string>("");
  const [largerThanMb, setLargerThanMb] = useState<string>("");
  const [hasError, setHasError] = useState(false);
  const [sidechain, setSidechain] = useState(false);
  // Plan is paired with the exact filter that produced it. Execute
  // uses this captured filter — never `buildFilter()` at click time —
  // so a filter change between Preview and Execute cannot widen the
  // prune to an unpreviewed set. The `useEffect` below clears both
  // when filter inputs change so the user must Preview again.
  const [plan, setPlan] = useState<{
    plan: PrunePlan;
    filter: PruneFilterInput;
  } | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  // Monotonic counter so a late preview response from a superseded
  // filter can't repopulate the plan with stale entries the user is
  // about to act on. Mirrors `useSessionSearch`'s requestSeqRef pattern.
  const previewSeqRef = useRef(0);

  const buildFilter = useCallback((): PruneFilterInput => {
    return {
      older_than_secs:
        olderThanDays.trim() !== ""
          ? Math.floor(Number(olderThanDays) * 86400)
          : null,
      larger_than_bytes:
        largerThanMb.trim() !== ""
          ? Math.floor(Number(largerThanMb) * 1_000_000)
          : null,
      project: [],
      has_error: hasError ? true : null,
      is_sidechain: sidechain ? true : null,
    };
  }, [olderThanDays, largerThanMb, hasError, sidechain]);

  const preview = useCallback(async () => {
    const mySeq = ++previewSeqRef.current;
    setErr(null);
    setLoading(true);
    // Snapshot the filter once so the plan and the filter that
    // produced it travel together.
    const filter = buildFilter();
    try {
      const p = await api.sessionPrunePlan(filter);
      // Discard the response if the user changed filters (or
      // re-clicked Preview) while we were waiting — the plan we'd
      // commit would not match the current filter row.
      if (mySeq !== previewSeqRef.current) return;
      setPlan({ plan: p, filter });
    } catch (e) {
      if (mySeq !== previewSeqRef.current) return;
      setErr(String(e));
      setPlan(null);
    } finally {
      if (mySeq === previewSeqRef.current) setLoading(false);
    }
  }, [buildFilter]);

  const execute = useCallback(async () => {
    if (!plan || plan.plan.entries.length === 0) return;
    setRunning(true);
    setErr(null);
    try {
      // Use the filter the plan was built from, not a fresh
      // buildFilter() — otherwise editing inputs after Preview would
      // execute a different cut than the user just inspected.
      const opId = await api.sessionPruneStart(plan.filter);
      onOpChange?.(opId);
      onTrashChanged?.();
    } catch (e) {
      setErr(String(e));
    } finally {
      setRunning(false);
    }
  }, [plan, onOpChange, onTrashChanged]);

  // Filter changes invalidate the prune preview. The slim
  // subsection owns its own plan and invalidates it on the same
  // signal via the `buildFilter` reference identity. Bumping
  // `previewSeqRef` here ensures any in-flight preview from the
  // prior filter is discarded on arrival.
  useEffect(() => {
    previewSeqRef.current++;
    setPlan(null);
  }, [olderThanDays, largerThanMb, hasError, sidechain]);

  const anyFilter =
    olderThanDays.trim() !== "" ||
    largerThanMb.trim() !== "" ||
    hasError ||
    sidechain;

  return (
    <section
      aria-label="Cleanup"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-16)",
        padding: "var(--sp-24)",
      }}
    >
      <div
        style={{
          display: "flex",
          gap: "var(--sp-12)",
          flexWrap: "wrap",
          alignItems: "center",
        }}
      >
        <Input
          glyph={NF.clock}
          type="number"
          placeholder="Older than (days)"
          value={olderThanDays}
          onChange={(e) => setOlderThanDays(e.target.value)}
          aria-label="Older than days"
          style={{ width: "var(--input-width-md)" }}
        />
        <Input
          glyph={NF.archive}
          type="number"
          placeholder="Larger than (MB)"
          value={largerThanMb}
          onChange={(e) => setLargerThanMb(e.target.value)}
          aria-label="Larger than MB"
          style={{ width: "var(--input-width-md)" }}
        />
        <FilterChip active={hasError} onToggle={() => setHasError((v) => !v)}>
          Errors
        </FilterChip>
        <FilterChip active={sidechain} onToggle={() => setSidechain((v) => !v)}>
          Agents
        </FilterChip>
        <div style={{ flex: 1 }} />
        <Button
          variant="ghost"
          onClick={preview}
          disabled={!anyFilter || loading}
          title={anyFilter ? undefined : "Pick at least one filter"}
        >
          {loading ? "Previewing…" : "Preview"}
        </Button>
        <Button
          variant="solid"
          onClick={execute}
          disabled={!plan || plan.plan.entries.length === 0 || running}
          title={
            !plan
              ? "Run Preview first"
              : plan.plan.entries.length === 0
                ? "Nothing matches the filter"
                : undefined
          }
        >
          {running ? "Pruning…" : "Prune → Trash"}
        </Button>
      </div>

      {err && (
        <div
          role="alert"
          style={{
            color: "var(--danger)",
            fontSize: "var(--fs-xs)",
          }}
        >
          {err}
        </div>
      )}

      {plan && (
        <CleanupPlanPreview
          testid="prune-preview"
          summaryText={
            `Plan · ${plan.plan.entries.length} file(s) · ${formatSize(plan.plan.total_bytes)}` +
            (plan.plan.entries.length === 0 ? " · nothing to prune" : "")
          }
          rows={plan.plan.entries.map((e) => ({
            id: e.file_path,
            leftText: e.file_path,
            rightText: formatSize(e.size_bytes),
          }))}
        />
      )}

      <SlimSubsection
        anyFilter={anyFilter}
        buildFilter={buildFilter}
        loading={loading}
        running={running}
        setLoading={setLoading}
        setRunning={setRunning}
        setErr={setErr}
        onOpChange={onOpChange}
        onTrashChanged={onTrashChanged}
      />

      {setToast && <SessionIndexRebuild setToast={setToast} />}
    </section>
  );
}

