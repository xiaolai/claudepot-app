import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { i18n } from "../../lib/i18n";
import { formatNumber } from "../../lib/intl";
import type { RetentionReport } from "../../api/cc-retention";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { ConfirmDialog } from "../../components/ConfirmDialog";
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
//  2. `0` is never a stop on the duration scale — and is no longer a
//     value at all. CC's floor is 1; a `0` on disk fails validation and
//     makes CC skip cleanup, so a user who once chose "stop saving" is
//     now accumulating transcripts forever while believing the
//     opposite. There is no "stop saving" control here any more,
//     because CC offers no settings-level way to do it (see
//     `claudepot_core::cc_retention`). The pane's job for that state is
//     to explain the inversion and offer the repair.
//  3. A long window is a BUFFER, not durability — the files still sit
//     in CC's cache directory under CC's rules. Say so, or a big
//     number reads as "solved".
//
// One rule added when rule 2 changed: **any action that lifts cleanup
// suppression confirms first.** While suppressed, the transcripts are
// protected by accident, and every route out of that state — a preset,
// or restoring the default — re-arms deletion on the whole backlog. A
// preset click is a one-tap destructive action in that state and
// nowhere else.

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

/** Resolve a preset's label at call time, so it follows a locale switch
 *  rather than freezing whatever language was active when it was
 *  stored. Falls back to the raw day count for a value not in PRESETS,
 *  which cannot happen today but must not render "undefined" if it
 *  ever does. */
function presetLabel(days: number): string {
  return PRESETS.find((p) => p.days === days)?.label ?? String(days);
}

function fmtDate(ms: number | null): string | null {
  if (ms == null) return null;
  return new Date(ms).toISOString().slice(0, 10);
}

/** The consequence block: what is scheduled to go, and what the counts
 *  do and do not cover. Extracted so the pane's own render function is
 *  an outline rather than the whole surface. */
function RiskSummary({ report }: { report: RetentionReport }) {
  const { t } = useTranslation("settings");
  const { state, risk } = report;
  const atRiskTotal = risk.already_deletable + risk.at_risk_within_horizon;
  const oldest = fmtDate(risk.oldest_ms);

  // Headline severity: anything already past the cutoff is live loss.
  // A legacy zero is NOT danger — nothing is being deleted in that
  // state. It is a warning, because what the user believes is happening
  // and what is happening have come apart.
  const severity =
    risk.already_deletable > 0
      ? "var(--danger)"
      : atRiskTotal > 0 || state.cleanup_suppressed
        ? "var(--warn)"
        : "var(--fg-muted)";

  const modeLine =
    state.mode === "cc_default"
      ? t("retention.modeDefault", { days: state.effective_days })
      : state.mode === "legacy_zero"
        ? t("retention.modeLegacyZero")
        : state.mode === "invalid"
          ? t("retention.modeInvalid", { days: state.configured_days })
          : t("retention.modeDays", { days: state.effective_days });

  return (
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
              // the raw number; `num` carries the grouped display form.
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
        {atRiskTotal === 0 && !risk.scan_incomplete && (
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
        {/* Both suppressed states protect transcripts by accident, but
            they need different sentences: `invalid` is a value to
            correct, `legacy_zero` is a control that used to work and
            was withdrawn upstream. */}
        {state.cleanup_suppressed && (
          <div style={{ color: "var(--warn)" }}>
            {state.mode === "legacy_zero"
              ? t("retention.risk.suppressedLegacyZero")
              : t("retention.risk.suppressed")}
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
        <div style={{ marginTop: "var(--sp-3)", color: "var(--fg-faint)" }}>
          {t("retention.scopeNote")}
        </div>
      </div>
    </div>
  );
}

/** Everything else `cleanupPeriodDays` ages out. The counts above are
 *  conversations only; without this the pane names the gap in prose but
 *  never quantifies it, which is a smaller version of the same
 *  over-claim. Entirely absent when nothing else is on the timer. */
function SweptPanel({ swept }: { swept: RetentionReport["swept_elsewhere"] }) {
  const { t } = useTranslation("settings");
  if (swept.dirs.length === 0) return null;
  return (
    <div>
      <SectionLabel>{t("retention.swept.title")}</SectionLabel>
      <div
        style={{
          marginTop: "var(--sp-8)",
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          lineHeight: "var(--lh-body)",
        }}
      >
        <div>{t("retention.swept.lead")}</div>
        {swept.dirs.map((d) => (
          <div
            key={d.rel}
            style={{
              marginTop: "var(--sp-3)",
              color: d.already_deletable > 0 ? "var(--warn)" : undefined,
            }}
          >
            {d.already_deletable > 0
              ? t("retention.swept.rowAtRisk", {
                  what: d.what,
                  dir: d.rel,
                  entries: formatNumber(d.entries),
                  deletable: formatNumber(d.already_deletable),
                })
              : t("retention.swept.row", {
                  what: d.what,
                  dir: d.rel,
                  entries: formatNumber(d.entries),
                })}
          </div>
        ))}
        {swept.scan_incomplete && (
          <div style={{ marginTop: "var(--sp-3)", color: "var(--warn)" }}>
            {t("retention.swept.incomplete")}
          </div>
        )}
        {swept.cache_dirs_skipped > 0 && (
          <div style={{ marginTop: "var(--sp-3)", color: "var(--fg-faint)" }}>
            {t("retention.swept.skipped", { count: swept.cache_dirs_skipped })}
          </div>
        )}
      </div>
    </div>
  );
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
  // Set only while cleanup is suppressed: the preset the user picked,
  // held until they confirm re-arming deletion. Null in every other
  // state, where a preset applies immediately as before.
  // Stores `days` only, never the resolved label. A label captured here
  // would freeze the language it was resolved in: switch locale while
  // the dialog is open and the confirm text keeps the old one, the same
  // class of bug as the module-level label constants i18n-plan warns
  // about. The label is derived from PRESETS at render time.
  const [confirmPresetDays, setConfirmPresetDays] = useState<number | null>(null);

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
      setConfirmPresetDays(null);
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

  // Severity, the mode line and the risk copy all moved into
  // <RiskSummary>; this function keeps only what the interactive
  // controls need.
  const { state, risk, is_durable_archive } = report;

  return (
    <div
      style={{
        display: "grid",
        gap: "var(--sp-24)",
        // tokens.css is the only place sizes are declared (design.md).
        maxWidth: "var(--content-cap-lg)",
      }}
    >
      <RiskSummary report={report} />
      <SweptPanel swept={report.swept_elsewhere} />

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
                onClick={() => {
                  // While cleanup is suppressed this button re-arms
                  // deletion of the entire backlog, so it stops being a
                  // preference and becomes a destructive action.
                  if (state.cleanup_suppressed) {
                    setConfirmPresetDays(p.days);
                    return;
                  }
                  void run(
                    () => api.retentionSet(p.days),
                    t("retention.keepToast", { label: p.label }),
                  );
                }}
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
        {/* Why there is no "stop saving transcripts" button here any
            more. Stated rather than silently removed: a user who set
            that once will come looking for it, and the honest answer is
            that Claude Code withdrew the capability, not that Claudepot
            hid it. */}
        <div
          style={{
            marginTop: "var(--sp-8)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {t("retention.noDisableNote")}
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

      {/* Only reachable while cleanup is suppressed — see the preset
          onClick. Elsewhere a preset is an ordinary preference and
          applies without a dialog. */}
      {confirmPresetDays !== null && (
        <ConfirmDialog
          title={t("retention.confirmRearm.title", {
            label: presetLabel(confirmPresetDays),
          })}
          body={
            <Trans
              ns="settings"
              i18nKey={
                risk.total_transcripts > 0
                  ? "retention.confirmRearm.bodyCount"
                  : "retention.confirmRearm.body"
              }
              components={{ strong: <strong /> }}
              values={{
                label: presetLabel(confirmPresetDays),
                num: formatNumber(risk.total_transcripts),
              }}
            />
          }
          confirmLabel={t("retention.confirmRearm.confirm")}
          confirmDanger
          onCancel={() => setConfirmPresetDays(null)}
          onConfirm={() =>
            void run(
              () => api.retentionSet(confirmPresetDays),
              t("retention.keepToast", { label: presetLabel(confirmPresetDays) }),
            )
          }
        />
      )}
    </div>
  );
}
