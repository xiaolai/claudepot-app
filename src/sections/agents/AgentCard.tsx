import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { AgentRunStatus, AgentSummaryDto } from "../../types";
import { formatElapsed } from "../../lib/elapsed";
import { RunHistoryPanel } from "./RunHistoryPanel";

interface Props {
  agent: AgentSummaryDto;
  /** A *mutation* is in flight on this card (edit / delete / toggle).
   *  Deliberately NOT "this agent is running" — the two were one boolean
   *  until 2026-08-18 and could not be told apart, so a 15-minute run
   *  and a 200ms toggle disabled the same controls for the same reason.
   *  Liveness is `run`, below, and it comes from the backend. */
  mutating: boolean;
  /** This agent's run state from the liveness poll: the in-flight run if
   *  any, plus the last completed one. `undefined` means the poll has
   *  not reported this agent yet — distinct from a status whose
   *  `in_flight` is null, which means "polled, nothing running". */
  runStatus?: AgentRunStatus;
  /** The liveness poll itself failed. Renders "can't determine" rather
   *  than idle — a failed read is not a clean result. */
  runsUnknown?: boolean;
  /** Increments when a run completes — RunHistoryPanel re-fetches. */
  runsRefreshKey: number;
  onRun: (id: string) => void;
  onEdit: (a: AgentSummaryDto) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onRemove: (a: AgentSummaryDto) => void;
  /** Open the review/install modal for a draft agent. */
  onReview: (a: AgentSummaryDto) => void;
}

/** Coarse "how long ago", reusing the elapsed formatter so a run that
 *  finished 14 minutes ago and one that ran for 14 minutes read in the
 *  same units. */
function relativeAgo(endedMs: number, nowMs: number): string {
  return formatElapsed(nowMs - endedMs);
}

export function AgentCard({
  agent,
  mutating,
  runStatus,
  runsUnknown = false,
  runsRefreshKey,
  onRun,
  onEdit,
  onToggle,
  onRemove,
  onReview,
}: Props) {
  const { t } = useTranslation("agents");
  const [open, setOpen] = useState(false);
  // A draft is inert — no scheduler artifact, never fires. The
  // footer below swaps the run/toggle controls for "Review &
  // install" because none of those actions are meaningful until a
  // human arms the agent.
  const isDraft = agent.lifecycle === "draft";

  const run = runStatus?.in_flight ?? undefined;
  const last = runStatus?.last ?? undefined;
  const isRunning = run?.state === "running";
  // Both reasons disable the controls, but they are different facts and
  // only one of them gets an inline explanation next to Run Now.
  const controlsLocked = mutating || isRunning;

  // Live elapsed. The 1s interval exists ONLY while a run is live —
  // render-if-nonzero applied to timers, not just to text. Without the
  // guard every card would hold a ticking interval forever.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!isRunning) return;
    const h = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(h);
  }, [isRunning]);

  const startedLabel = run
    ? new Date(run.started_ms).toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
      })
    : "";

  return (
    <article
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-3)",
        background: "var(--bg-raised)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          gap: "var(--sp-8)",
          flexWrap: "wrap",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            color: "var(--fg)",
          }}
        >
          {agent.display_name || agent.name}
        </h3>
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {agent.name}
        </span>
        <span style={{ flex: 1 }} />
        {isDraft ? (
          <Tag tone="warn">{t("card.status.draft")}</Tag>
        ) : (
          <Tag tone={agent.enabled ? "ok" : "ghost"}>
            {agent.enabled
              ? t("card.status.enabled")
              : t("card.status.disabled")}
          </Tag>
        )}
      </header>

      {isDraft && (
        <p
          role="note"
          style={{
            margin: 0,
            padding: "var(--sp-6) var(--sp-8)",
            border: "var(--bw-hair) solid var(--warn)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            background: "var(--bg)",
          }}
        >
          {t("card.draftNote")}
        </p>
      )}

      {agent.description && (
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
          }}
        >
          {agent.description}
        </p>
      )}

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "max-content 1fr",
          gap: "var(--sp-4) var(--sp-12)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        <span style={{ color: "var(--fg-faint)" }}>{t("card.labels.cron")}</span>
        <span style={{ fontFamily: "var(--font-mono)" }}>
          {agent.cron ?? "—"}
        </span>
        <span style={{ color: "var(--fg-faint)" }}>{t("card.labels.cwd")}</span>
        <span style={{ fontFamily: "var(--font-mono)" }}>{agent.cwd}</span>
        <span style={{ color: "var(--fg-faint)" }}>{t("card.labels.binary")}</span>
        <span>
          {agent.binary_kind === "first_party"
            ? t("card.binaryFirstParty")
            : t("card.binaryRoute", { id: agent.binary_route_id ?? "?" })}
          {agent.model && (
            <span style={{ color: "var(--fg-faint)" }}> · {agent.model}</span>
          )}
        </span>
        <span style={{ color: "var(--fg-faint)" }}>
          {t("card.labels.permissions")}
        </span>
        <span>
          {agent.permission_mode}
          {agent.allowed_tools.length > 0 && (
            <span
              style={{
                color: "var(--fg-faint)",
                fontFamily: "var(--font-mono)",
                marginLeft: "var(--sp-4)",
              }}
            >
              [{agent.allowed_tools.join(", ")}]
            </span>
          )}
        </span>
        {agent.max_budget_usd !== null && (
          <>
            <span style={{ color: "var(--fg-faint)" }}>
              {t("card.labels.budget")}
            </span>
            <span>${agent.max_budget_usd.toFixed(2)}</span>
          </>
        )}
      </div>

      {/* The ambient tier for this agent — exactly one row, per
          design.md's signal budget. The status-bar chip carries the
          COUNT; identity and duration live here, on the thing you are
          looking at. Render-if-nonzero: absent entirely when idle. */}
      {run && (
        <div
          role="status"
          style={{
            fontSize: "var(--fs-xs)",
            color: isRunning ? "var(--fg-muted)" : "var(--warn)",
          }}
        >
          {isRunning
            ? t("card.run.running", {
                elapsed: formatElapsed(now - run.started_ms),
                started: startedLabel,
              })
            : /* Not "failed" and not "running" — the run left no result
                 and nothing is working on it. Saying which of those it
                 was would be a guess. */
              t("card.run.interrupted")}
        </div>
      )}
      {runsUnknown && !run && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--warn)" }}>
          {t("card.run.unknown")}
        </div>
      )}
      {/* The three idle states from the plan's §2.2 table. Only rendered
          when nothing is in flight — a live run supersedes history, and
          showing both would put two ambient rows on one card. */}
      {!isDraft && !run && !runsUnknown && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {!last
            ? t("card.run.never")
            : last.exit_code === 0
              ? t("card.run.lastOk", {
                  ago: relativeAgo(last.ended_ms, now),
                  took: formatElapsed(last.duration_ms),
                })
              : /* The exit code is the only honest thing we have here;
                   naming a cause ("budget exhausted") would require
                   parsing the run's stdout, and guessing one is how a
                   status surface starts lying. */
                t("card.run.lastFailed", {
                  ago: relativeAgo(last.ended_ms, now),
                  code: last.exit_code,
                })}
        </div>
      )}

      <footer
        style={{
          display: "flex",
          gap: "var(--sp-6)",
          flexWrap: "wrap",
        }}
      >
        {isDraft ? (
          <>
            {/* A draft has exactly one primary action: review the
                spec and arm it. Run / Edit / Toggle / runs history
                are all meaningless for an inert record. */}
            <Button
              variant="solid"
              onClick={() => onReview(agent)}
              disabled={controlsLocked}
            >
              {t("card.actions.reviewInstall")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onRemove(agent)}
              disabled={controlsLocked}
            >
              {t("card.actions.delete")}
            </Button>
          </>
        ) : (
          <>
            <Button
              variant="solid"
              onClick={() => onRun(agent.id)}
              disabled={controlsLocked}
            >
              {t("card.actions.runNow")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onEdit(agent)}
              disabled={controlsLocked}
            >
              {t("card.actions.edit")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onToggle(agent.id, !agent.enabled)}
              disabled={controlsLocked}
            >
              {agent.enabled
                ? t("card.actions.disable")
                : t("card.actions.enable")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onRemove(agent)}
              disabled={controlsLocked}
            >
              {t("card.actions.delete")}
            </Button>
            <span style={{ flex: 1 }} />
            <Button variant="ghost" onClick={() => setOpen((o) => !o)}>
              {open
                ? t("card.actions.hideRuns")
                : t("card.actions.showRuns")}
            </Button>
          </>
        )}
      </footer>

      {/* design.md: a disabled control states its reason inline, next to
          the button — never in a tooltip. Only the RUNNING lock is
          explained; `mutating` is a sub-second state nobody needs a
          sentence about. */}
      {!isDraft && isRunning && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
          {t("card.run.runNowDisabled", { started: startedLabel })}
        </div>
      )}

      {!isDraft && open && (
        <RunHistoryPanel
          agentId={agent.id}
          refreshKey={runsRefreshKey}
        />
      )}
    </article>
  );
}
