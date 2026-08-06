import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { AgentSummaryDto } from "../../types";
import { RunHistoryPanel } from "./RunHistoryPanel";

interface Props {
  agent: AgentSummaryDto;
  busy: boolean;
  /** Increments when a run completes — RunHistoryPanel re-fetches. */
  runsRefreshKey: number;
  onRun: (id: string) => void;
  onEdit: (a: AgentSummaryDto) => void;
  onToggle: (id: string, enabled: boolean) => void;
  onRemove: (a: AgentSummaryDto) => void;
  /** Open the review/install modal for a draft agent. */
  onReview: (a: AgentSummaryDto) => void;
}

export function AgentCard({
  agent,
  busy,
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
            fontFamily: "var(--ff-mono)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-3)",
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
            color: "var(--fg-2)",
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
            color: "var(--fg-2)",
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
          color: "var(--fg-2)",
        }}
      >
        <span style={{ color: "var(--fg-3)" }}>{t("card.labels.cron")}</span>
        <span style={{ fontFamily: "var(--ff-mono)" }}>
          {agent.cron ?? "—"}
        </span>
        <span style={{ color: "var(--fg-3)" }}>{t("card.labels.cwd")}</span>
        <span style={{ fontFamily: "var(--ff-mono)" }}>{agent.cwd}</span>
        <span style={{ color: "var(--fg-3)" }}>{t("card.labels.binary")}</span>
        <span>
          {agent.binary_kind === "first_party"
            ? t("card.binaryFirstParty")
            : t("card.binaryRoute", { id: agent.binary_route_id ?? "?" })}
          {agent.model && (
            <span style={{ color: "var(--fg-3)" }}> · {agent.model}</span>
          )}
        </span>
        <span style={{ color: "var(--fg-3)" }}>
          {t("card.labels.permissions")}
        </span>
        <span>
          {agent.permission_mode}
          {agent.allowed_tools.length > 0 && (
            <span
              style={{
                color: "var(--fg-3)",
                fontFamily: "var(--ff-mono)",
                marginLeft: "var(--sp-4)",
              }}
            >
              [{agent.allowed_tools.join(", ")}]
            </span>
          )}
        </span>
        {agent.max_budget_usd !== null && (
          <>
            <span style={{ color: "var(--fg-3)" }}>
              {t("card.labels.budget")}
            </span>
            <span>${agent.max_budget_usd.toFixed(2)}</span>
          </>
        )}
      </div>

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
              disabled={busy}
            >
              {t("card.actions.reviewInstall")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onRemove(agent)}
              disabled={busy}
            >
              {t("card.actions.delete")}
            </Button>
          </>
        ) : (
          <>
            <Button
              variant="solid"
              onClick={() => onRun(agent.id)}
              disabled={busy}
            >
              {t("card.actions.runNow")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onEdit(agent)}
              disabled={busy}
            >
              {t("card.actions.edit")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onToggle(agent.id, !agent.enabled)}
              disabled={busy}
            >
              {agent.enabled
                ? t("card.actions.disable")
                : t("card.actions.enable")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => onRemove(agent)}
              disabled={busy}
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

      {!isDraft && open && (
        <RunHistoryPanel
          agentId={agent.id}
          refreshKey={runsRefreshKey}
        />
      )}
    </article>
  );
}
