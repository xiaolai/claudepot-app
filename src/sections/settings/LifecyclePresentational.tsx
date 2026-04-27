// Tiny presentational primitives shared by DisabledArtifactList,
// ArtifactTrashList, and ArtifactLifecyclePane. Sharded out so each
// table file is independently readable and the pane stays under the
// loc-guardian limit.

import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

export function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section>
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          gap: "var(--sp-12)",
          marginBottom: "var(--sp-8)",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {title}
        </h3>
        {action}
      </header>
      {children}
    </section>
  );
}

export function Table({ children }: { children: React.ReactNode }) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-sm)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      {children}
    </table>
  );
}

export function Th({
  children,
  ...rest
}: React.ThHTMLAttributes<HTMLTableCellElement>) {
  return (
    <th
      {...rest}
      style={{
        textAlign: "left",
        padding: "var(--sp-6) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        fontWeight: 500,
        color: "var(--fg-muted)",
        fontSize: "var(--fs-2xs)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      {children}
    </th>
  );
}

export function Td({
  children,
  muted,
  align,
}: {
  children: React.ReactNode;
  muted?: boolean;
  align?: "left" | "right";
}) {
  return (
    <td
      style={{
        padding: "var(--sp-6) var(--sp-12)",
        textAlign: align ?? "left",
        color: muted ? "var(--fg-muted)" : "var(--fg)",
      }}
    >
      {children}
    </td>
  );
}

export function rowStyle(): React.CSSProperties {
  return { borderBottom: "var(--bw-hair) solid var(--line)" };
}

export function Empty({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-16) var(--sp-20)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-sm)",
        color: danger ? "var(--danger)" : "var(--fg-muted)",
      }}
    >
      <Glyph g={NF.info} style={{ marginRight: "var(--sp-6)" }} />
      {children}
    </div>
  );
}
