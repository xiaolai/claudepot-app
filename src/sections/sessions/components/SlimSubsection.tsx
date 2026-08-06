import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api";
import { renderError } from "../../../lib/i18n-error";
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
  const { t } = useTranslation("sessions");
  const [stripImages, setStripImages] = useState(false);
  const [stripDocuments, setStripDocuments] = useState(false);
  const [slimPlan, setSlimPlan] = useState<BulkSlimPlan | null>(null);
  // Monotonic counter so a late slim-preview response from a
  // superseded filter or flag change can't repopulate `slimPlan`
  // with stale entries the user is about to act on.
  const previewSeqRef = useRef(0);

  // Toggling a slim flag must invalidate a stale slim plan, but
  // leave any prune preview the host might be showing untouched.
  useEffect(() => {
    previewSeqRef.current++;
    setSlimPlan(null);
  }, [stripImages, stripDocuments]);

  // Filter changes upstream also invalidate the slim plan. The host
  // calls `onFilterChanged` via the `buildFilter` identity changing,
  // but a more direct signal is cheap: invalidate whenever
  // `buildFilter`'s reference changes (it's a useCallback over the
  // filter inputs).
  useEffect(() => {
    previewSeqRef.current++;
    setSlimPlan(null);
  }, [buildFilter]);

  const anySlimFlag = stripImages || stripDocuments;

  const previewSlim = useCallback(async () => {
    const mySeq = ++previewSeqRef.current;
    setErr(null);
    setLoading(true);
    try {
      const p = await api.sessionSlimPlanAll(buildFilter(), {
        drop_tool_results_over_bytes: 1 << 20,
        exclude_tools: [],
        strip_images: stripImages,
        strip_documents: stripDocuments,
      });
      // Discard the response if the user changed filters/flags
      // (or re-clicked Preview) while we were waiting — the plan
      // we'd commit would not match the current inputs.
      if (mySeq !== previewSeqRef.current) return;
      setSlimPlan(p);
    } catch (e) {
      if (mySeq !== previewSeqRef.current) return;
      setErr(renderError(e));
      setSlimPlan(null);
    } finally {
      if (mySeq === previewSeqRef.current) setLoading(false);
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
      setErr(renderError(e));
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
        {t("cleanup.slimHeading")}
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
          {t("cleanup.stripImages")}
        </FilterChip>
        <FilterChip
          active={stripDocuments}
          onToggle={() => setStripDocuments((v) => !v)}
        >
          {t("cleanup.stripDocuments")}
        </FilterChip>
        <div style={{ flex: 1 }} />
        <Button
          variant="ghost"
          onClick={previewSlim}
          disabled={!anyFilter || !anySlimFlag || loading}
          title={
            !anyFilter
              ? t("cleanup.pickFilterAbove")
              : !anySlimFlag
                ? t("cleanup.pickImagesOrDocs")
                : undefined
          }
        >
          {loading ? t("cleanup.previewing") : t("cleanup.previewSlim")}
        </Button>
        <Button
          variant="solid"
          onClick={executeSlim}
          disabled={!slimPlan || slimPlan.entries.length === 0 || running}
          title={
            !slimPlan
              ? t("cleanup.runPreviewSlimFirst")
              : slimPlan.entries.length === 0
                ? t("cleanup.nothingMatchesFilter")
                : undefined
          }
        >
          {running ? t("cleanup.slimming") : t("cleanup.slimToTrash")}
        </Button>
      </div>

      {slimPlan && (
        <CleanupPlanPreview
          testid="slim-preview"
          marginTop="var(--sp-8)"
          summaryText={
            t("cleanup.slimPlanSummary", {
              n: slimPlan.entries.length,
              size: formatSize(slimPlan.total_bytes_saved),
            }) +
            (slimPlan.total_image_redacts > 0
              ? ` · ${t("cleanup.slimImages", { n: slimPlan.total_image_redacts })}`
              : "") +
            (slimPlan.total_document_redacts > 0
              ? ` · ${t("cleanup.slimDocs", { n: slimPlan.total_document_redacts })}`
              : "") +
            (slimPlan.entries.length === 0
              ? ` · ${t("cleanup.nothingToSlim")}`
              : "")
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
                {t("cleanup.couldNotScan", {
                  count: slimPlan.failed_to_plan.length,
                })}
                <ul style={{ margin: "var(--sp-4) 0 0", paddingInlineStart: "var(--sp-16)" }}>
                  {slimPlan.failed_to_plan.slice(0, 10).map(([p, err]) => (
                    <li key={p} title={err}>
                      {p}
                    </li>
                  ))}
                  {slimPlan.failed_to_plan.length > 10 && (
                    <li style={{ color: "var(--fg-faint)" }}>
                      {t("cleanup.andNMore", {
                        n: slimPlan.failed_to_plan.length - 10,
                      })}
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
