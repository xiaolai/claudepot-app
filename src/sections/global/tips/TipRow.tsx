// One tip row. Collapsed shows: prose (or "Variant A / B" header
// if conditional), category, seen-status, last-seen, trigger
// summary. Expanded reveals: both variants with condition labels,
// experiment metadata, raw isRelevant source under disclosure.

import { useState } from "react";
import { Glyph } from "../../../components/primitives/Glyph";
import { Tag } from "../../../components/primitives/Tag";
import { NF } from "../../../icons";
import type { RenderedTip } from "../../../types/cc-tips";

export function TipRow({ tip }: { tip: RenderedTip }) {
  const [open, setOpen] = useState(false);
  const hasVariants = tip.prose_b !== null;

  const seenBadge =
    tip.seen_status === "seen" ? (
      <Tag tone="ok">
        <Glyph g={NF.check} />
        seen
      </Tag>
    ) : (
      <Tag tone="neutral">never seen</Tag>
    );

  const summary = hasVariants
    ? `${tip.prose} • ${tip.prose_b ?? ""}`
    : tip.prose;

  return (
    <div
      style={{
        borderTop: "var(--bw-hair) solid var(--line)",
        padding: "var(--sp-10) var(--sp-14)",
      }}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        style={{
          all: "unset",
          cursor: "pointer",
          width: "100%",
          display: "grid",
          gridTemplateColumns: "auto 1fr",
          gap: "var(--sp-10)",
          alignItems: "start",
        }}
      >
        <Glyph g={open ? NF.chevronD : NF.chevronR} />
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
          <div
            style={{
              fontSize: "var(--fs-sm)",
              color: "var(--fg)",
              lineHeight: 1.4,
              overflow: "hidden",
              textOverflow: "ellipsis",
              display: "-webkit-box",
              WebkitLineClamp: open ? "unset" : 2,
              WebkitBoxOrient: "vertical",
            }}
          >
            {open ? null : <span>{summary}</span>}
          </div>
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              display: "flex",
              gap: "var(--sp-8)",
              flexWrap: "wrap",
              alignItems: "center",
            }}
          >
            <Tag tone="neutral">{tip.category_label}</Tag>
            {seenBadge}
            <span>
              {tip.last_seen
                ? tip.seen_status === "seen"
                  ? `seen ${tip.last_seen.relative}`
                  : null
                : null}
            </span>
            <span>
              {tip.cooldown_sessions !== null
                ? `cooldown ${tip.cooldown_sessions}`
                : null}
            </span>
            <span>{tip.trigger_summary}</span>
            {tip.experiment_flag && (
              <Tag tone="warn">experiment: {tip.experiment_flag}</Tag>
            )}
          </div>
        </div>
      </button>
      {open && (
        <div
          style={{
            marginTop: "var(--sp-8)",
            paddingLeft: "calc(var(--sp-10) + var(--sp-16))",
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-8)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
          }}
        >
          {hasVariants ? (
            <>
              <Variant
                label={tip.condition_label ?? "Variant A"}
                prose={tip.prose}
              />
              <Variant
                label={tip.condition_label_b ?? "Variant B"}
                prose={tip.prose_b ?? ""}
              />
            </>
          ) : (
            <p style={{ margin: 0, lineHeight: 1.5 }}>{tip.prose}</p>
          )}
          {tip.last_seen && tip.seen_status === "seen" && (
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
            >
              Last seen at startup #{tip.last_seen.startup_count_when_seen}
              {tip.last_seen.exact_unknown
                ? " (before snapshot history)"
                : ` (${tip.last_seen.relative})`}
            </p>
          )}
          {tip.relevance_source && (
            <details>
              <summary
                style={{
                  cursor: "pointer",
                  fontSize: "var(--fs-2xs)",
                  color: "var(--fg-faint)",
                }}
              >
                Show advanced trigger logic
              </summary>
              <pre
                style={{
                  marginTop: "var(--sp-4)",
                  padding: "var(--sp-6) var(--sp-8)",
                  fontSize: "var(--fs-2xs)",
                  background: "var(--bg-sunken)",
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--rad-2)",
                  overflowX: "auto",
                  maxHeight: 200,
                }}
              >
                {tip.relevance_source.trim()}
              </pre>
            </details>
          )}
        </div>
      )}
    </div>
  );
}

function Variant({ label, prose }: { label: string; prose: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
      <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
        {label}
      </span>
      <p style={{ margin: 0, lineHeight: 1.5 }}>{prose}</p>
    </div>
  );
}
