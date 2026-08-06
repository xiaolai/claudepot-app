import { useTranslation } from "react-i18next";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import type { PendingJournalsSummary } from "../types";

/**
 * Status-bar chip for actionable rename journals (plan §7.5).
 * Replaces the global `PendingJournalsBanner` that floated above
 * the content pane.
 *
 * Tone-aware:
 * - any stale entry (≥24h) → `warn` variant (amber border + ink)
 * - pending only (<24h, dead lock) → neutral chip
 * - running entries are excluded — they show in the running-ops
 *   chip alongside; nagging twice would violate "one signal per
 *   surface".
 *
 * Renders nothing when there's nothing actionable.
 */
export function PendingJournalsChip({
  summary,
  onOpen,
}: {
  summary: PendingJournalsSummary | null;
  onOpen: () => void;
}) {
  const { t } = useTranslation("components");
  if (!summary) return null;
  const total = summary.pending + summary.stale;
  if (total <= 0) return null;

  const hasStale = summary.stale > 0;
  const noun = t("chips.pending", { n: total });
  const ariaDetail = hasStale
    ? summary.pending === 0
      ? t("chips.pendingAllStale", { total })
      : t("chips.pendingSomeStale", { stale: summary.stale, total })
    : t("chips.pendingDeadLock", { count: total });

  return (
    <button
      type="button"
      className={`statusbar-chip${hasStale ? " warn" : ""}`}
      onClick={onOpen}
      aria-label={t("chips.pendingAria", {
        count: total,
        noun,
        detail: ariaDetail,
      })}
      title={t("chips.pendingTitle")}
    >
      <Glyph
        g={hasStale ? NF.warn : NF.wrench}
        style={{ fontSize: "var(--fs-2xs)" }}
      />
      <span>{noun}</span>
    </button>
  );
}
