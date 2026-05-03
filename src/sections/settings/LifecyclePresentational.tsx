// Section + Empty helpers shared by DisabledArtifactList,
// ArtifactTrashList, and ArtifactLifecyclePane. The table primitives
// (Table/Th/Td/Tr) live in `components/primitives/Table.tsx` and are
// imported directly by the list files.

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
