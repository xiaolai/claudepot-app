import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { RetentionReport } from "../../api/cc-retention";
import { Button } from "../../components/primitives/Button";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import { toastError } from "../../lib/toastError";

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

/** Stops on the scale. `0` is deliberately absent — see rule 2. */
const PRESETS: { days: number; label: string }[] = [
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
  { days: 365, label: "1 year" },
  { days: 3650, label: "10 years" },
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
      setLoadError(e instanceof Error ? e.message : String(e));
      toastError(pushToast, "Retention load failed", e);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (fn: () => Promise<RetentionReport>, msg: string) => {
    setBusy(true);
    try {
      setReport(await fn());
      pushToast("info", msg);
    } catch (e) {
      toastError(pushToast, "Change retention", e);
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
          Couldn&rsquo;t read Claude Code&rsquo;s retention setting, so this
          pane cannot tell you what is scheduled for deletion.
        </div>
        <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-xs)" }}>
          {loadError}
        </div>
        <Button variant="outline" onClick={() => void refresh()}>
          Try again
        </Button>
      </div>
    );
  }

  if (!report) {
    return <div style={{ color: "var(--fg-faint)" }}>Loading…</div>;
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
      ? `${state.effective_days} days — Claude Code's default, not a choice you made`
      : state.mode === "persistence_disabled"
        ? "Transcripts are not being saved at all"
        : state.mode === "invalid"
          ? `Invalid value (${state.configured_days}) — Claude Code's settings schema rejects it`
          : `${state.effective_days} days`;

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
        <SectionLabel>Transcript retention</SectionLabel>
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
              {risk.already_deletable.toLocaleString()} transcript
              {risk.already_deletable === 1 ? "" : "s"} will be deleted the
              next time Claude Code starts.
            </div>
          )}
          {risk.at_risk_within_horizon > 0 && (
            <div style={{ color: "var(--warn)" }}>
              {risk.at_risk_within_horizon.toLocaleString()} more cross the
              cutoff within {risk.horizon_days} days.
            </div>
          )}
          {/* Never reassure on an incomplete scan — a permissions
              failure must not read as "all clear". */}
          {risk.scan_incomplete && (
            <div style={{ color: "var(--warn)" }}>
              Part of Claude Code&rsquo;s transcript folder couldn&rsquo;t be
              read, so these counts are a floor, not a total.
            </div>
          )}
          {atRiskTotal === 0 &&
            !risk.scan_incomplete &&
            state.mode !== "persistence_disabled" && (
              <div>
                Nothing is scheduled for deletion.
                {/* render-if-nonzero: never ship "0 transcripts". */}
                {risk.total_transcripts > 0 && (
                  <> {risk.total_transcripts.toLocaleString()} transcripts on this machine.</>
                )}
              </div>
            )}
          {state.cleanup_suppressed && (
            <div style={{ color: "var(--warn)" }}>
              Claude Code is skipping cleanup entirely because this value
              makes its settings file invalid. That is protecting your
              transcripts by accident — correct the value rather than
              restoring the default, which would re-arm deletion.
            </div>
          )}
          {oldest && (
            <div style={{ marginTop: "var(--sp-3)" }}>
              Oldest surviving conversation: {oldest}.
            </div>
          )}
          <div style={{ marginTop: "var(--sp-3)", color: "var(--fg-faint)" }}>
            Claude Code does this silently — it never reports what it
            deleted.
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
          Cleanup only removes the {risk.total_transcripts.toLocaleString()}{" "}
          top-level session transcripts.{" "}
          {risk.nested_immortal.toLocaleString()} nested files (subagent
          runs, tool results) are never removed, so the folder keeps
          growing while conversations are deleted — disk usage cannot
          reveal this loss.
        </div>
      )}

      <div>
        <SectionLabel>Keep transcripts for</SectionLabel>
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
                    `Claude Code will keep transcripts for ${p.label}.`,
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
            Saving retention…
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
            A longer window buys time; it is not a backup. These files
            still live in Claude Code's cache directory under Claude
            Code's rules, and they grow by roughly a gigabyte a week on
            heavy use.
          </div>
        )}
      </div>

      <div>
        <SectionLabel>Danger zone</SectionLabel>
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
              Restore Claude Code's default
            </Button>
          )}
          {state.mode !== "persistence_disabled" && (
            <Button
              variant="outline"
              danger
              disabled={busy}
              onClick={() => setConfirmDisable(true)}
            >
              Stop saving transcripts entirely
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
          Both re-enable deletion. Neither can be undone for transcripts
          already removed.
        </div>
      </div>

      {confirmClear && (
        <ConfirmDialog
          title="Restore Claude Code's default retention?"
          body={
            <>
              Retention returns to <strong>30 days</strong>. Claude Code
              will delete every transcript older than that the next time
              it starts
              {risk.total_transcripts > 0 && (
                <>
                  {" "}
                  — {risk.total_transcripts.toLocaleString()} transcripts
                  are currently kept
                </>
              )}
              . This cannot be undone.
            </>
          }
          confirmLabel="Restore default"
          confirmDanger
          onCancel={() => setConfirmClear(false)}
          onConfirm={() =>
            void run(
              () => api.retentionClear(),
              "Retention restored to Claude Code's 30-day default.",
            )
          }
        />
      )}

      {confirmDisable && (
        <ConfirmDangerousAction
          title="Stop saving transcripts entirely?"
          consequences={
            <>
              <p>
                This writes <code>cleanupPeriodDays: 0</code>, which does
                two things:
              </p>
              <ul>
                <li>Claude Code stops writing transcripts for new sessions.</li>
                <li>
                  Every existing transcript
                  {risk.total_transcripts > 0 && (
                    <> ({risk.total_transcripts.toLocaleString()} on this machine)</>
                  )}{" "}
                  is deleted the next time Claude Code starts.
                </li>
              </ul>
              <p>
                Session resume, search, and every Claudepot surface built
                on transcripts stop working. This cannot be undone.
              </p>
            </>
          }
          confirmLabel="Stop saving and delete"
          typeToConfirm="delete my transcripts"
          onCancel={() => setConfirmDisable(false)}
          onConfirm={() =>
            void run(
              () => api.retentionDisablePersistence(),
              "Transcript persistence disabled.",
            )
          }
        />
      )}
    </div>
  );
}
