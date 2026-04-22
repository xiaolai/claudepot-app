import { useCallback, useEffect, useState } from "react";
import { api } from "../../../api";
import { Button } from "../../../components/primitives/Button";
import { FilterChip } from "../../../components/primitives/FilterChip";
import type { BulkSlimPlan, PruneFilterInput } from "../../../types";
import { formatSize } from "../../projects/format";
import { CleanupPlanPreview } from "./CleanupPlanPreview";

/**
 * "Reclaim image tokens" subsection of the Cleanup pane. Owns its
 * own slim flags + plan state but shares the global loading/running/
 * error indicators with its host so the parent's single error banner
 * still surfaces slim failures.
 *
 * Lifted out of `CleanupPane.tsx` so the host stays focused on the
 * prune flow plus the shared filter inputs.
 */
export function SlimSubsection({
  anyFilter,
  buildFilter,
  loading,
  running,
  setLoading,
  setRunning,
  setErr,
  onOpChange,
  onTrashChanged,
}: {
  /** True iff the user has picked at least one prune-side filter
   * input. The slim Preview button is disabled without that. */
  anyFilter: boolean;
  /** Factory for the current `PruneFilterInput`. Called on every
   * preview/execute so the slim run uses the latest filter the user
   * has typed in the prune row above. */
  buildFilter: () => PruneFilterInput;
  /** Shared "preview is in flight" flag — disables both prune and
   * slim preview buttons so the user can't fire two scans at once. */
  loading: boolean;
  /** Shared "execute is in flight" flag — same reasoning. */
  running: boolean;
  setLoading: (v: boolean) => void;
  setRunning: (v: boolean) => void;
  setErr: (msg: string | null) => void;
  onOpChange?: (opId: string | null) => void;
  onTrashChanged?: () => void;
}) {
  const [stripImages, setStripImages] = useState(false);
  const [stripDocuments, setStripDocuments] = useState(false);
  const [slimPlan, setSlimPlan] = useState<BulkSlimPlan | null>(null);

  // Toggling a slim flag must invalidate a stale slim plan, but
  // leave any prune preview the host might be showing untouched.
  useEffect(() => {
    setSlimPlan(null);
  }, [stripImages, stripDocuments]);

  // Filter changes upstream also invalidate the slim plan. The host
  // calls `onFilterChanged` via the `buildFilter` identity changing,
  // but a more direct signal is cheap: invalidate whenever
  // `buildFilter`'s reference changes (it's a useCallback over the
  // filter inputs).
  useEffect(() => {
    setSlimPlan(null);
  }, [buildFilter]);

  const anySlimFlag = stripImages || stripDocuments;

  const previewSlim = useCallback(async () => {
    setErr(null);
    setLoading(true);
    try {
      const p = await api.sessionSlimPlanAll(buildFilter(), {
        drop_tool_results_over_bytes: 1 << 20,
        exclude_tools: [],
        strip_images: stripImages,
        strip_documents: stripDocuments,
      });
      setSlimPlan(p);
    } catch (e) {
      setErr(String(e));
      setSlimPlan(null);
    } finally {
      setLoading(false);
    }
  }, [buildFilter, stripImages, stripDocuments, setErr, setLoading]);

  const executeSlim = useCallback(async () => {
    if (!slimPlan || slimPlan.entries.length === 0) return;
    setRunning(true);
    setErr(null);
    try {
      const opId = await api.sessionSlimStartAll(buildFilter(), {
        drop_tool_results_over_bytes: 1 << 20,
        exclude_tools: [],
        strip_images: stripImages,
        strip_documents: stripDocuments,
      });
      onOpChange?.(opId);
      onTrashChanged?.();
    } catch (e) {
      setErr(String(e));
    } finally {
      setRunning(false);
    }
  }, [
    slimPlan,
    buildFilter,
    stripImages,
    stripDocuments,
    onOpChange,
    onTrashChanged,
    setErr,
    setRunning,
  ]);

  return (
    <div
      data-testid="slim-subsection"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        paddingTop: "var(--sp-16)",
        borderTop: "var(--bw-hair) solid var(--line)",
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        Reclaim image tokens
      </div>
      <div
        style={{
          display: "flex",
          gap: "var(--sp-12)",
          flexWrap: "wrap",
          alignItems: "center",
        }}
      >
        <FilterChip
          active={stripImages}
          onToggle={() => setStripImages((v) => !v)}
        >
          Strip images
        </FilterChip>
        <FilterChip
          active={stripDocuments}
          onToggle={() => setStripDocuments((v) => !v)}
        >
          Strip documents
        </FilterChip>
        <div style={{ flex: 1 }} />
        <Button
          variant="ghost"
          onClick={previewSlim}
          disabled={!anyFilter || !anySlimFlag || loading}
          title={
            !anyFilter
              ? "Pick at least one filter above"
              : !anySlimFlag
                ? "Pick images and/or documents"
                : undefined
          }
        >
          {loading ? "Previewing…" : "Preview slim"}
        </Button>
        <Button
          variant="solid"
          onClick={executeSlim}
          disabled={!slimPlan || slimPlan.entries.length === 0 || running}
          title={
            !slimPlan
              ? "Run Preview slim first"
              : slimPlan.entries.length === 0
                ? "Nothing matches the filter"
                : undefined
          }
        >
          {running ? "Slimming…" : "Slim → Trash"}
        </Button>
      </div>

      {slimPlan && (
        <CleanupPlanPreview
          testid="slim-preview"
          marginTop="var(--sp-8)"
          summaryText={
            `Slim · ${slimPlan.entries.length} file(s) · ${formatSize(slimPlan.total_bytes_saved)} saved` +
            (slimPlan.total_image_redacts > 0
              ? ` · ${slimPlan.total_image_redacts} images`
              : "") +
            (slimPlan.total_document_redacts > 0
              ? ` · ${slimPlan.total_document_redacts} docs`
              : "") +
            (slimPlan.entries.length === 0 ? " · nothing to slim" : "")
          }
          rows={slimPlan.entries.map((e) => ({
            id: e.file_path,
            leftText: e.file_path,
            rightText: formatSize(e.plan.bytes_saved),
          }))}
          extrasFooter={
            slimPlan.failed_to_plan.length > 0 ? (
              <div
                data-testid="slim-failed-to-plan"
                style={{
                  padding: "var(--sp-8) var(--sp-16)",
                  borderTop: "var(--bw-hair) solid var(--line)",
                  fontSize: "var(--fs-xs)",
                  color: "var(--danger)",
                  background: "var(--bg-sunken)",
                }}
              >
                Could not scan {slimPlan.failed_to_plan.length} session
                {slimPlan.failed_to_plan.length === 1 ? "" : "s"}:
                <ul style={{ margin: "var(--sp-4) 0 0", paddingInlineStart: "var(--sp-16)" }}>
                  {slimPlan.failed_to_plan.slice(0, 10).map(([p, err]) => (
                    <li key={p} title={err}>
                      {p}
                    </li>
                  ))}
                  {slimPlan.failed_to_plan.length > 10 && (
                    <li style={{ color: "var(--fg-faint)" }}>
                      … and {slimPlan.failed_to_plan.length - 10} more
                    </li>
                  )}
                </ul>
              </div>
            ) : null
          }
        />
      )}
    </div>
  );
}
