import { useState, useCallback } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { formatSize } from "../../lib/format";
import { renderError } from "../../lib/i18n-error";
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
  const { t } = useTranslation("projects");
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
      pushToast("error", renderError(e, t("gc.previewFailedScope")));
    } finally {
      setBusy(false);
    }
  }, [days, daysValid, pushToast, t]);

  const execute = useCallback(async () => {
    if (!daysValid || !previewMatchesInput) return;
    setBusy(true);
    try {
      const r = await api.repairGc(days, false);
      setResult(null);
      setPreviewDays(null);
      pushToast(
        "info",
        t("gc.execToast", {
          journals: r.removed_journals,
          snapshots: r.removed_snapshots,
          size: formatSize(r.bytes_freed),
        }),
      );
    } catch (e) {
      pushToast("error", renderError(e, t("gc.failedScope")));
    } finally {
      setBusy(false);
    }
  }, [days, daysValid, previewMatchesInput, pushToast, t]);

  return (
    <section className="maintenance-section">
      <div className="maintenance-section-header">
        <h2>{t("gc.heading")}</h2>
      </div>
      <p className="muted maintenance-desc">
        {t("gc.desc")}
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
          {t("gc.olderThan")}
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
        <span className="muted small">{t("gc.days")}</span>
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
              ? t("gc.previewTitle")
              : t("gc.rangeTitle", { min: GC_DAYS_MIN, max: GC_DAYS_MAX })
          }
        >
          {t("gc.preview")}
        </Button>
        <Button
          variant="solid"
          danger
          onClick={execute}
          disabled={busy || !result || !daysValid || !previewMatchesInput}
          title={
            !daysValid
              ? t("gc.rangeTitle", { min: GC_DAYS_MIN, max: GC_DAYS_MAX })
              : !result || !previewMatchesInput
                ? t("gc.runPreviewFirst")
                : undefined
          }
        >
          {t("gc.execute")}
        </Button>
        {!daysValid && (
          <span
            className="muted small"
            style={{ color: "var(--bad)" }}
            role="status"
          >
            {t("gc.daysRange", { min: GC_DAYS_MIN, max: GC_DAYS_MAX })}
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
          <Trans
            ns="projects"
            i18nKey="gc.wouldRemove"
            values={{
              journals: result.removed_journals,
              snapshots: result.removed_snapshots,
            }}
            components={{
              j: <strong style={{ color: "var(--fg)" }} />,
              s: <strong style={{ color: "var(--fg)" }} />,
            }}
          />
        </div>
      )}
    </section>
  );
}
