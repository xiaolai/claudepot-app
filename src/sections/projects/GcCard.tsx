import { useState, useCallback } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import type { GcOutcome } from "../../types";

/**
 * Time-based GC for abandoned rename journals + old recovery
 * snapshots. Moved from the Settings → Cleanup tab as part of the
 * C-1 E consolidation — project-domain cleanup belongs in the
 * project-domain maintenance view. Preview is idempotent and
 * mandatory before Execute. Execute is irreversible.
 */
const GC_DAYS_MIN = 1;
const GC_DAYS_MAX = 365;

export function GcCard({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  // Audit T2 H#3: `days` is forwarded to an irreversible GC. Track the
  // raw input as the source of truth so empty / NaN / out-of-range
  // values disable the actions instead of silently coercing to 0 (or
  // any other unsafe value) and running outside the advertised
  // 1–365 window.
  const [days, setDays] = useState<number>(30);
  const [busy, setBusy] = useState(false);
  // `result` is paired with the `previewDays` it was computed for. If
  // the user changes the days input after Preview, the preview
  // becomes stale and Execute must not run against the new threshold
  // — otherwise an unpreviewed value could delete more than the
  // preview promised. Clearing result on input change forces a fresh
  // preview before Execute is enabled.
  const [previewDays, setPreviewDays] = useState<number | null>(null);
  const [result, setResult] = useState<GcOutcome | null>(null);

  const daysValid =
    Number.isFinite(days) && days >= GC_DAYS_MIN && days <= GC_DAYS_MAX;
  const previewMatchesInput = previewDays !== null && previewDays === days;

  const dryRun = useCallback(async () => {
    if (!daysValid) return;
    setBusy(true);
    try {
      const r = await api.repairGc(days, true);
      setResult(r);
      setPreviewDays(days);
    } catch (e) {
      pushToast("error", `GC preview failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [days, daysValid, pushToast]);

  const execute = useCallback(async () => {
    if (!daysValid || !previewMatchesInput) return;
    setBusy(true);
    try {
      const r = await api.repairGc(days, false);
      setResult(null);
      setPreviewDays(null);
      pushToast(
        "info",
        `GC: removed ${r.removed_journals} journals, ${r.removed_snapshots} snapshots, freed ${formatBytes(
          r.bytes_freed,
        )}`,
      );
    } catch (e) {
      pushToast("error", `GC failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [days, daysValid, previewMatchesInput, pushToast]);

  return (
    <section className="maintenance-section">
      <div className="maintenance-section-header">
        <h2>Garbage-collect old repair data</h2>
      </div>
      <p className="muted maintenance-desc">
        Remove abandoned rename journals and recovery snapshots older
        than the threshold. Preview first — the execute step is
        irreversible.
      </p>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          marginBottom: "var(--sp-12)",
        }}
      >
        <label htmlFor="gc-days" className="muted small">
          Older than
        </label>
        <input
          id="gc-days"
          type="number"
          min={GC_DAYS_MIN}
          max={GC_DAYS_MAX}
          value={Number.isFinite(days) ? days : ""}
          onChange={(e) => {
            setDays(e.target.valueAsNumber);
            // Days changed → previously-previewed result no longer
            // describes what Execute would do. Clear it so the user
            // must Preview again before Execute lights back up.
            setResult(null);
            setPreviewDays(null);
          }}
          aria-invalid={!daysValid}
          style={{
            width: "var(--sp-72)",
            padding: "var(--sp-4) var(--sp-6)",
            fontSize: "var(--fs-sm)",
            fontFamily: "var(--font)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            background: "var(--bg)",
            color: "var(--fg)",
            fontVariantNumeric: "tabular-nums",
          }}
        />
        <span className="muted small">days</span>
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        <Button
          variant="ghost"
          onClick={dryRun}
          disabled={busy || !daysValid}
          title={
            daysValid
              ? "Preview what GC would remove"
              : `Enter a value between ${GC_DAYS_MIN} and ${GC_DAYS_MAX}`
          }
        >
          Preview
        </Button>
        <Button
          variant="solid"
          danger
          onClick={execute}
          disabled={busy || !result || !daysValid || !previewMatchesInput}
          title={
            !daysValid
              ? `Enter a value between ${GC_DAYS_MIN} and ${GC_DAYS_MAX}`
              : !result || !previewMatchesInput
                ? "Run Preview first"
                : undefined
          }
        >
          Execute GC
        </Button>
        {!daysValid && (
          <span
            className="muted small"
            style={{ color: "var(--bad)" }}
            role="status"
          >
            Days must be {GC_DAYS_MIN}–{GC_DAYS_MAX}.
          </span>
        )}
      </div>
      {result && (
        <div
          style={{
            marginTop: "var(--sp-12)",
            padding: "var(--sp-10) var(--sp-12)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
          }}
        >
          Would remove:{" "}
          <strong style={{ color: "var(--fg)" }}>
            {result.removed_journals}
          </strong>{" "}
          journals,{" "}
          <strong style={{ color: "var(--fg)" }}>
            {result.removed_snapshots}
          </strong>{" "}
          snapshots
        </div>
      )}
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
