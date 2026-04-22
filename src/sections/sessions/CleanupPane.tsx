import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { FilterChip } from "../../components/primitives/FilterChip";
import { Input } from "../../components/primitives/Input";
import { NF } from "../../icons";
import type { BulkSlimPlan, PruneFilterInput, PrunePlan } from "../../types";

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
  const [stripImages, setStripImages] = useState(false);
  const [stripDocuments, setStripDocuments] = useState(false);
  const [plan, setPlan] = useState<PrunePlan | null>(null);
  const [slimPlan, setSlimPlan] = useState<BulkSlimPlan | null>(null);
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

  // Slim plan + execute. Dry-run matches prune's UX: Preview fills
  // the plan, Execute dispatches the bulk op. Defaults in the core
  // `SlimOpts` (drop_tool_results_over_bytes: 1 MiB) hold even when
  // the user hasn't set them.
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
  }, [buildFilter, stripImages, stripDocuments]);

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
  }, [slimPlan, buildFilter, stripImages, stripDocuments, onOpChange, onTrashChanged]);

  // Split invalidation: prune preview only depends on the filter,
  // slim preview depends on the filter AND the slim flags. Toggling
  // a slim flag must NOT wipe a valid prune preview.
  useEffect(() => {
    setPlan(null);
    setSlimPlan(null);
  }, [olderThanDays, largerThanMb, hasError, sidechain]);

  useEffect(() => {
    setSlimPlan(null);
  }, [stripImages, stripDocuments]);

  const anyFilter =
    olderThanDays.trim() !== "" ||
    largerThanMb.trim() !== "" ||
    hasError ||
    sidechain;
  const anySlimFlag = stripImages || stripDocuments;

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
        <div
          data-testid="prune-preview"
          style={{
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              padding: "var(--sp-12) var(--sp-16)",
              background: "var(--bg-sunken)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            Plan · {plan.entries.length} file(s) · {formatSize(plan.total_bytes)}
            {plan.entries.length === 0 ? " · nothing to prune" : ""}
          </div>
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {plan.entries.slice(0, 50).map((e) => (
              <li
                key={e.file_path}
                style={{
                  padding: "var(--sp-8) var(--sp-16)",
                  borderBottom: "var(--bw-hair) solid var(--line)",
                  fontSize: "var(--fs-sm)",
                  display: "grid",
                  gridTemplateColumns: "1fr auto",
                  gap: "var(--sp-16)",
                }}
              >
                <span
                  title={e.file_path}
                  style={{
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                  }}
                >
                  {e.file_path}
                </span>
                <span
                  style={{
                    fontVariantNumeric: "tabular-nums",
                    color: "var(--fg-muted)",
                  }}
                >
                  {formatSize(e.size_bytes)}
                </span>
              </li>
            ))}
          </ul>
          {plan.entries.length > 50 && (
            <div
              style={{
                padding: "var(--sp-8) var(--sp-16)",
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              … and {plan.entries.length - 50} more
            </div>
          )}
        </div>
      )}

      {/* Bulk slim — "reclaim image tokens" path. Uses the same filter
          above; dispatches session_slim_plan_all / _start_all. */}
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
          <div
            data-testid="slim-preview"
            style={{
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              overflow: "hidden",
              marginTop: "var(--sp-8)",
            }}
          >
            <div
              style={{
                padding: "var(--sp-12) var(--sp-16)",
                background: "var(--bg-sunken)",
                fontSize: "var(--fs-xs)",
                color: "var(--fg-muted)",
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
              }}
            >
              Slim · {slimPlan.entries.length} file(s) ·{" "}
              {formatSize(slimPlan.total_bytes_saved)} saved
              {slimPlan.total_image_redacts > 0
                ? ` · ${slimPlan.total_image_redacts} images`
                : ""}
              {slimPlan.total_document_redacts > 0
                ? ` · ${slimPlan.total_document_redacts} docs`
                : ""}
              {slimPlan.entries.length === 0 ? " · nothing to slim" : ""}
            </div>
            <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
              {slimPlan.entries.slice(0, 50).map((e) => (
                <li
                  key={e.file_path}
                  style={{
                    padding: "var(--sp-8) var(--sp-16)",
                    borderBottom: "var(--bw-hair) solid var(--line)",
                    fontSize: "var(--fs-sm)",
                    display: "grid",
                    gridTemplateColumns: "1fr auto",
                    gap: "var(--sp-16)",
                  }}
                >
                  <span
                    title={e.file_path}
                    style={{
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {e.file_path}
                  </span>
                  <span
                    style={{
                      fontVariantNumeric: "tabular-nums",
                      color: "var(--fg-muted)",
                    }}
                  >
                    {formatSize(e.plan.bytes_saved)}
                  </span>
                </li>
              ))}
            </ul>
            {slimPlan.entries.length > 50 && (
              <div
                style={{
                  padding: "var(--sp-8) var(--sp-16)",
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                … and {slimPlan.entries.length - 50} more
              </div>
            )}
            {slimPlan.failed_to_plan.length > 0 && (
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
            )}
          </div>
        )}
      </div>
    </section>
  );
}

function formatSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1000) return `${(bytes / 1000).toFixed(1)} KB`;
  return `${bytes} B`;
}
