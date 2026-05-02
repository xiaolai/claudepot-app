import type { ReactNode } from "react";
import type { TemplateScopeDto } from "../../types";

/**
 * The four plain-English scope statements — `reads`, `writes`,
 * `could_change`, `network` — rendered verbatim as the trust-
 * boundary surface in the install dialog.
 *
 * Templates author each line once; we never paraphrase here.
 * The whole point of this block is to make the install
 * commitment explicit BEFORE the user clicks Install.
 */
export function ScopeStatements({ scope }: { scope: TemplateScopeDto }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "auto 1fr",
        columnGap: "var(--sp-12)",
        rowGap: "var(--sp-6)",
        padding: "var(--sp-8) var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg-sunken)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <Row label="Reads">{scope.reads}</Row>
      <Row label="Writes">{scope.writes}</Row>
      <Row label="Changes">{scope.could_change}</Row>
      <Row label="Network">{scope.network}</Row>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <span
        className="mono-cap"
        style={{
          color: "var(--fg-faint)",
          fontSize: "var(--fs-2xs)",
          alignSelf: "center",
        }}
      >
        {label}
      </span>
      <span style={{ color: "var(--fg)", lineHeight: 1.45 }}>{children}</span>
    </>
  );
}
