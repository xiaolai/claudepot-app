import type { ReactNode } from "react";
import { BackAffordance } from "../../../components/primitives/BackAffordance";
import { Glyph } from "../../../components/primitives/Glyph";
import { Tag } from "../../../components/primitives/Tag";
import { NF } from "../../../icons";
import type { SessionRow } from "../../../types";
import { modelBadge, projectBasename, shortSessionId } from "../format";

/**
 * Compact session-header layout — single ~40px row that takes over
 * once the user scrolls past 16px in the transcript. Carries only
 * the identity bits a reader still needs at a glance: breadcrumb,
 * one-line title, error/model badges, plus the right-aligned Reveal
 * + kebab nodes the orchestrator passes through.
 *
 * Pure presentation. No menu state, no scroll detection — both live
 * in `SessionDetailHeader`.
 */
export function SessionDetailHeaderCompact({
  row,
  title,
  onBack,
  revealNode,
  kebabNode,
}: {
  row: SessionRow;
  title: string;
  onBack?: () => void;
  revealNode: ReactNode;
  kebabNode: ReactNode;
}) {
  const project = projectBasename(row.project_path) || row.slug;
  return (
    <div
      style={{
        padding: "var(--sp-8) var(--sp-28)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
        background: "var(--bg)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-12)",
        minHeight: "var(--sp-40)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          flexShrink: 0,
        }}
      >
        {onBack ? (
          <BackAffordance
            label={project}
            onClick={onBack}
            title={`Back to session list for ${project}`}
          />
        ) : (
          <span>{project}</span>
        )}
        <Glyph g={NF.chevronR} style={{ fontSize: "var(--fs-3xs)" }} />
        <span className="mono" title={row.session_id}>
          {shortSessionId(row.session_id)}
        </span>
      </div>

      <h3
        style={{
          flex: 1,
          margin: 0,
          fontSize: "var(--fs-sm)",
          fontWeight: 500,
          color: "var(--fg)",
          letterSpacing: "var(--ls-normal)",
          textTransform: "none",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
          minWidth: 0,
        }}
        title={row.first_user_prompt ?? title}
      >
        {title}
      </h3>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
        }}
      >
        {row.has_error && (
          <Tag tone="warn" glyph={NF.warn}>
            error
          </Tag>
        )}
        {row.models.length > 0 && (
          <Tag tone="accent" title={row.models.join(", ")}>
            {modelBadge(row.models)}
          </Tag>
        )}
        {revealNode}
        {kebabNode}
      </div>
    </div>
  );
}
