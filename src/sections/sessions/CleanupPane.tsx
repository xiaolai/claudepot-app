import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { FilterChip } from "../../components/primitives/FilterChip";
import { Input } from "../../components/primitives/Input";
import { NF } from "../../icons";
import type { PruneFilterInput, PrunePlan } from "../../types";
import { formatSize } from "../projects/format";
import { CleanupPlanPreview } from "./components/CleanupPlanPreview";
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
}: {
  /** Called with the op_id when a prune starts. */
  onOpChange?: (opId: string | null) => void;
  /** Called after a prune is dispatched so the parent can nudge refreshes. */
  onTrashChanged?: () => void;
}) {
  const [olderThanDays, setOlderThanDays] = useState<string>("");
  const [largerThanMb, setLargerThanMb] = useState<string>("");
  const [hasError, setHasError] = useState(false);
  const [sidechain, setSidechain] = useState(false);
  const [plan, setPlan] = useState<PrunePlan | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

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
    setErr(null);
    setLoading(true);
    try {
      const p = await api.sessionPrunePlan(buildFilter());
      setPlan(p);
    } catch (e) {
      setErr(String(e));
      setPlan(null);
    } finally {
      setLoading(false);
    }
  }, [buildFilter]);

  const execute = useCallback(async () => {
    if (!plan || plan.entries.length === 0) return;
    setRunning(true);
    setErr(null);
    try {
      const opId = await api.sessionPruneStart(buildFilter());
      onOpChange?.(opId);
      onTrashChanged?.();
    } catch (e) {
      setErr(String(e));
    } finally {
      setRunning(false);
    }
  }, [plan, buildFilter, onOpChange, onTrashChanged]);

  // Filter changes invalidate the prune preview. The slim
  // subsection owns its own plan and invalidates it on the same
  // signal via the `buildFilter` reference identity.
  useEffect(() => {
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
          style={{ width: 210 }}
        />
        <Input
          glyph={NF.archive}
          type="number"
          placeholder="Larger than (MB)"
          value={largerThanMb}
          onChange={(e) => setLargerThanMb(e.target.value)}
          aria-label="Larger than MB"
          style={{ width: 210 }}
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
          disabled={!plan || plan.entries.length === 0 || running}
          title={
            !plan
              ? "Run Preview first"
              : plan.entries.length === 0
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
            `Plan · ${plan.entries.length} file(s) · ${formatSize(plan.total_bytes)}` +
            (plan.entries.length === 0 ? " · nothing to prune" : "")
          }
          rows={plan.entries.map((e) => ({
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
    </section>
  );
}

