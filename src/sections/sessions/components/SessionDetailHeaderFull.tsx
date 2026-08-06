import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { BackAffordance } from "../../../components/primitives/BackAffordance";
import { CopyButton } from "../../../components/CopyButton";
import { Glyph } from "../../../components/primitives/Glyph";
import { Tag } from "../../../components/primitives/Tag";
import { NF } from "../../../icons";
import type { SessionRow } from "../../../types";
import {
  estimatedRateHint,
  formatCost,
  formatUsd,
  type PricedCost,
} from "../../../costs";
import { maybeRedact } from "../../../lib/redactSecrets";
import { formatRelativeTime, formatSize } from "../../projects/format";
import {
  bestTimestampMs,
  formatTokens,
  modelBadge,
  projectBasename,
  shortSessionId,
} from "../format";

/**
 * Full session-header layout — breadcrumb, two-line title, tag row,
 * metadata row, and the action footer. Rendered by
 * `SessionDetailHeader` when the user is at the top of the
 * transcript; the compact sibling takes over once they scroll.
 *
 * Pure presentation. The orchestrator owns the kebab popover state,
 * the price-table fetch, and supplies the right-aligned action
 * buttons via `revealNode` / `kebabNode`, so this file never imports
 * `ContextMenu` and never touches the `pricingGet` API surface.
 */
export function SessionDetailHeaderFull({
  row,
  title,
  cost,
  onBack,
  revealNode,
  kebabNode,
}: {
  row: SessionRow;
  title: string;
  /** API-equivalent cost for the session, or `null` when the price
   * table is still loading or has no entries for the row's models.
   * Computed once in the orchestrator so it survives the compact↔
   * full layout transitions without re-fetching pricing. */
  cost: PricedCost | null;
  onBack?: () => void;
  revealNode: ReactNode;
  kebabNode: ReactNode;
}) {
  const { t } = useTranslation("sessions");
  const lastTs = bestTimestampMs(row.last_ts, row.last_modified_ms);
  const firstTs = row.first_ts ? Date.parse(row.first_ts) : null;
  const project = projectBasename(row.project_path) || row.slug;

  return (
    <div
      style={{
        padding: "var(--sp-20) var(--sp-28) var(--sp-14)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
        background: "var(--bg)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          marginBottom: "var(--sp-6)",
        }}
      >
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {onBack ? (
            <BackAffordance
              label={project}
              onClick={onBack}
              title={t("detail.backToSessionList", { project })}
            />
          ) : (
            <span>{project}</span>
          )}
          <Glyph g={NF.chevronR} style={{ fontSize: "var(--fs-3xs)" }} />
          <span className="mono" title={row.session_id}>
            {shortSessionId(row.session_id)}
          </span>
          <CopyButton text={row.session_id} />
        </div>
      </div>

      <h3
        style={{
          margin: 0,
          fontSize: "var(--fs-md-lg)",
          fontWeight: 600,
          color: "var(--fg)",
          letterSpacing: "var(--ls-normal)",
          textTransform: "none",
          overflow: "hidden",
          textOverflow: "ellipsis",
          display: "-webkit-box",
          WebkitLineClamp: 2,
          WebkitBoxOrient: "vertical",
        }}
        title={maybeRedact(row.first_user_prompt) ?? undefined}
      >
        {title}
      </h3>

      <div
        style={{
          marginTop: "var(--sp-10)",
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-8)",
        }}
      >
        {row.has_error && (
          <Tag tone="warn" glyph={NF.warn}>
            {t("detail.tagError")}
          </Tag>
        )}
        {row.is_sidechain && <Tag tone="ghost">{t("detail.tagAgent")}</Tag>}
        {row.models.length > 0 && (
          <Tag tone="accent" title={row.models.join(", ")}>
            {modelBadge(row.models)}
          </Tag>
        )}
        {row.git_branch && (
          <Tag tone="neutral" glyph={NF.branch}>
            {row.git_branch}
          </Tag>
        )}
        {row.cc_version && (
          <Tag tone="ghost">{t("detail.tagCc", { version: row.cc_version })}</Tag>
        )}
        {row.tokens.total > 0 && (
          <Tag
            tone="neutral"
            title={t("detail.tokensTooltip", {
              input: row.tokens.input,
              output: row.tokens.output,
              read: row.tokens.cache_read,
              write: row.tokens.cache_creation,
            })}
          >
            {t("viewer.tok", { tokens: formatTokens(row.tokens.total) })}
          </Tag>
        )}
        {cost !== null && cost.usd > 0 && (
          <Tag
            tone="neutral"
            title={
              t("detail.costTooltip", { usd: formatUsd(cost.usd) }) +
              (cost.confidence === "family_estimate" ? ` ${estimatedRateHint()}` : "")
            }
          >
            {t("detail.costOnApi", { cost: formatCost(cost) })}
          </Tag>
        )}
        {row.message_count > 0 && (
          <Tag tone="neutral">
            {t("detail.turns", { count: row.message_count })}
          </Tag>
        )}
        <Tag tone="ghost">{formatSize(row.file_size_bytes)}</Tag>
      </div>

      <div
        style={{
          marginTop: "var(--sp-10)",
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-12) var(--sp-16)",
          alignItems: "center",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-xs)",
        }}
      >
        <span
          title={row.project_path}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            maxWidth: "100%",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          <Glyph g={NF.folder} style={{ fontSize: "var(--fs-2xs)" }} />
          <span
            className="mono"
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {row.project_path}
          </span>
          <CopyButton text={row.project_path} />
        </span>
        {firstTs != null && (
          <span title={row.first_ts ?? ""}>
            {t("detail.started", { time: formatRelativeTime(firstTs) })}
          </span>
        )}
        {lastTs != null && (
          <span title={row.last_ts ?? ""}>
            {t("detail.lastEvent", { time: formatRelativeTime(lastTs) })}
          </span>
        )}
      </div>

      <div
        style={{
          marginTop: "var(--sp-14)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        {revealNode}
        {kebabNode}
      </div>
    </div>
  );
}
