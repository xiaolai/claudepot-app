import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { i18n } from "../../lib/i18n";
import { formatNumber } from "../../lib/intl";
import type { RetentionReport } from "../../api/cc-retention";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import { renderError, toastError } from "../../lib/i18n-error";
import { SkeletonList } from "../../components/primitives/Skeleton";

// Settings → Retention.
//
// Claude Code deletes transcripts older than `cleanupPeriodDays`
// (default 30) on every launch, and says nothing: the cleanup routine
// computes an exact count of what it unlinked and its caller discards
// the number. Nothing in CC's UI mentions the setting. This pane is the
// only place a user can find out their history is on a timer.
//
// Three design rules carried from the plan (P0b):
//
//  1. Lead with the consequence on THIS machine, not the policy.
//     Documentation loses to ignorance; a count that ticks does not.
//  2. `0` is never a stop on the duration scale. It means "delete
//     everything and stop persisting", so it lives behind a
//     type-to-confirm gate and is worded as what it does.
//  3. A long window is a BUFFER, not durability — the files still sit
//     in CC's cache directory under CC's rules. Say so, or a big
//     number reads as "solved".

/** Stops on the scale. `0` is deliberately absent — see rule 2.
 *  Labels resolve through the catalog at access time so a locale
 *  change re-renders them without this module needing React. */
const PRESETS: { days: number; label: string }[] = [
  { days: 30,
    get label() { return i18n.t("retention.preset30", { ns: "settings" }); } },
  { days: 90,
    get label() { return i18n.t("retention.preset90", { ns: "settings" }); } },
  { days: 365,
    get label() { return i18n.t("retention.preset365", { ns: "settings" }); } },
  { days: 3650,
    get label() { return i18n.t("retention.preset3650", { ns: "settings" }); } },
];

function fmtDate(ms: number | null): string | null {
  if (ms == null) return null;
  return new Date(ms).toISOString().slice(0, 10);
}

export function RetentionPane({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [report, setReport] = useState<RetentionReport | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [confirmDisable, setConfirmDisable] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setReport(await api.retentionReport());
      setLoadError(null);
    } catch (e) {
      // A pane that sits on "Loading…" forever after a failed toast
      // reads as "still working", which in this pane specifically
      // implies "still safe". Fail visibly instead.
      setLoadError(renderError(e));
      toastError(pushToast, t("retention.loadFailed"), e);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (fn: () => Promise<RetentionReport>, msg: string) => {
    setBusy(true);
    try {
      setReport(await fn());
      pushToast("info", msg);
    } catch (e) {
      toastError(pushToast, t("retention.changeFailed"), e);
    } finally {
      setBusy(false);
      setConfirmClear(false);
      setConfirmDisable(false);
    }
  };

  if (loadError) {
    return (
      <div style={{ display: "grid", gap: "var(--sp-12)", justifyItems: "start" }}>
        <div style={{ color: "var(--danger)", fontSize: "var(--fs-sm)" }}>
          {t("retention.loadErrorLead")}
        </div>
        <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-xs)" }}>
          {loadError}
        </div>
        <Button variant="outline" onClick={() => void refresh()}>
          {t("retention.tryAgain")}
        </Button>
      </div>
    );
  }

  if (!report) {
    // A shaped placeholder, not one line: this pane reports what is
    // scheduled for deletion, and a surface that collapses to a single
    // grey row reads as "nothing to report" rather than "not loaded".
    return <SkeletonList rows={3} label={t("shared.loading")} />;
  }

  const { state, risk, is_durable_archive } = report;
  const atRiskTotal = risk.already_deletable + risk.at_risk_within_horizon;
  const oldest = fmtDate(risk.oldest_ms);

  // Headline severity: anything already past the cutoff is live loss.
  const severity =
    state.mode === "persistence_disabled" || risk.already_deletable > 0
      ? "var(--danger)"
      : atRiskTotal > 0
        ? "var(--warn)"
        : "var(--fg-muted)";

  const modeLine =
    state.mode === "cc_default"
      ? t("retention.modeDefault", { days: state.effective_days })
      : state.mode === "persistence_disabled"
        ? t("retention.modeDisabled")
        : state.mode === "invalid"
          ? t("retention.modeInvalid", { days: state.configured_days })
          : t("retention.modeDays", { days: state.effective_days });

  return (
    <div
      style={{
        display: "grid",
        gap: "var(--sp-24)",
        // tokens.css is the only place sizes are declared (design.md).
        maxWidth: "var(--content-cap-lg)",
      }}
    >
      <div>
        <SectionLabel>{t("retention.sectionTitle")}</SectionLabel>
        <div
          style={{
            fontSize: "var(--fs-md)",
            color: severity,
            marginTop: "var(--sp-6)",
          }}
        >
          {modeLine}
        </div>

        {/* Consequence first. Render-if-nonzero: when nothing is at
            risk this collapses to the single reassuring line below. */}
        <div
          style={{
            marginTop: "var(--sp-8)",
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {risk.already_deletable > 0 && (
            <div style={{ color: "var(--danger)" }}>
              {t("retention.risk.deletable", {
                // `count` drives i18next's plural selection and must be
                // the raw number; `num` carries the grouped display form
                // (`1,234`). Both are required — this key was already
                // correct; `horizon` and `totalOnMachine` below were not.
                count: risk.already_deletable,
                num: formatNumber(risk.already_deletable),
              })}
            </div>
          )}
          {risk.at_risk_within_horizon > 0 && (
            <div style={{ color: "var(--warn)" }}>
              {t("retention.risk.horizon", {
                count: risk.at_risk_within_horizon,
                num: formatNumber(risk.at_risk_within_horizon),
                days: risk.horizon_days,
              })}
            </div>
          )}
          {/* Never reassure on an incomplete scan — a permissions
              failure must not read as "all clear". */}
          {risk.scan_incomplete && (
            <div style={{ color: "var(--warn)" }}>
              {t("retention.risk.scanIncomplete")}
            </div>
          )}
          {atRiskTotal === 0 &&
            !risk.scan_incomplete &&
            state.mode !== "persistence_disabled" && (
              <div>
                {t("retention.risk.nothing")}
                {/* render-if-nonzero: never ship "0 transcripts". */}
                {risk.total_transcripts > 0 && (
                  <>
                    {" "}
                    {t("retention.risk.totalOnMachine", {
                      count: risk.total_transcripts,
                      num: formatNumber(risk.total_transcripts),
                    })}
                  </>
                )}
              </div>
            )}
          {state.cleanup_suppressed && (
            <div style={{ color: "var(--warn)" }}>
              {t("retention.risk.suppressed")}
            </div>
          )}
          {oldest && (
            <div style={{ marginTop: "var(--sp-3)" }}>
              {t("retention.risk.oldest", { date: oldest })}
            </div>
          )}
          <div style={{ marginTop: "var(--sp-3)", color: "var(--fg-faint)" }}>
            {t("retention.risk.silent")}
          </div>
        </div>
      </div>

      {/* Why the folder keeps growing while history shrinks. Only shown
          when there is actually a nested pile to explain. */}
      {risk.nested_immortal > 0 && (
        <div
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {t("retention.nestedNote", {
            total: formatNumber(risk.total_transcripts),
            nested: formatNumber(risk.nested_immortal),
          })}
        </div>
      )}

      <div>
        <SectionLabel>{t("retention.keepFor")}</SectionLabel>
        <div
          style={{
            display: "flex",
            gap: "var(--sp-8)",
            flexWrap: "wrap",
            marginTop: "var(--sp-8)",
          }}
        >
          {PRESETS.map((p) => {
            const active =
              state.mode === "explicit" && state.configured_days === p.days;
            return (
              <Button
                key={p.days}
                variant={active ? "solid" : "outline"}
                disabled={busy}
                aria-pressed={active}
                onClick={() =>
                  void run(
                    () => api.retentionSet(p.days),
                    t("retention.keepToast", { label: p.label }),
                  )
                }
              >
                {p.label}
              </Button>
            );
          })}
        </div>
        {/* design.md: a disabled control states its reason inline,
            next to the button — not in a tooltip. */}
        {busy && (
          <div
            role="status"
            aria-live="polite"
            style={{
              marginTop: "var(--sp-6)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
            }}
          >
            {t("retention.savingNote")}
          </div>
        )}
        {!is_durable_archive && (
          <div
            style={{
              marginTop: "var(--sp-8)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              lineHeight: "var(--lh-body)",
            }}
          >
            {t("retention.bufferNote")}
          </div>
        )}
      </div>

      <div>
        <SectionLabel>{t("retention.dangerZone")}</SectionLabel>
        <div
          style={{
            display: "flex",
            gap: "var(--sp-8)",
            flexWrap: "wrap",
            marginTop: "var(--sp-8)",
          }}
        >
          {state.mode !== "cc_default" && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => setConfirmClear(true)}
            >
              {t("retention.restoreDefaultBtn")}
            </Button>
          )}
          {state.mode !== "persistence_disabled" && (
            <Button
              variant="outline"
              danger
              disabled={busy}
              onClick={() => setConfirmDisable(true)}
            >
              {t("retention.stopSavingBtn")}
            </Button>
          )}
        </div>
        <div
          style={{
            marginTop: "var(--sp-6)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {t("retention.dangerNote")}
        </div>
      </div>

      {confirmClear && (
        <ConfirmDialog
          title={t("retention.confirmRestore.title")}
          body={
            risk.total_transcripts > 0 ? (
              <Trans
                ns="settings"
                i18nKey="retention.confirmRestore.bodyCount"
                components={{ strong: <strong /> }}
                values={{ num: formatNumber(risk.total_transcripts) }}
              />
            ) : (
              <Trans
                ns="settings"
                i18nKey="retention.confirmRestore.body"
                components={{ strong: <strong /> }}
              />
            )
          }
          confirmLabel={t("retention.confirmRestore.confirm")}
          confirmDanger
          onCancel={() => setConfirmClear(false)}
          onConfirm={() =>
            void run(() => api.retentionClear(), t("retention.restoredToast"))
          }
        />
      )}

      {confirmDisable && (
        <ConfirmDangerousAction
          title={t("retention.confirmDisable.title")}
          consequences={
            <>
              <p>
                <Trans
                  ns="settings"
                  i18nKey="retention.confirmDisable.intro"
                  components={{ code: <code /> }}
                />
              </p>
              <ul>
                <li>{t("retention.confirmDisable.li1")}</li>
                <li>
                  {risk.total_transcripts > 0
                    ? t("retention.confirmDisable.li2Count", {
                        num: formatNumber(risk.total_transcripts),
                      })
                    : t("retention.confirmDisable.li2")}
                </li>
              </ul>
              <p>{t("retention.confirmDisable.outro")}</p>
            </>
          }
          confirmLabel={t("retention.confirmDisable.confirm")}
          typeToConfirm={t("retention.confirmDisable.phrase")}
          onCancel={() => setConfirmDisable(false)}
          onConfirm={() =>
            void run(
              () => api.retentionDisablePersistence(),
              t("retention.disabledToast"),
            )
          }
        />
      )}
    </div>
  );
}
