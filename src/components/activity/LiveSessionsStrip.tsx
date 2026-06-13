// "Live" strip at the top of the Activities tab (WI-L4).
//
// Renders one `LiveSessionCard` per active session from the
// existing live aggregate (`useSessionLive`). When no sessions
// are live, renders a small empty-state placeholder.

import { useSessionLive } from "../../hooks/useSessionLive";
import { SectionLabel } from "../primitives/SectionLabel";
import { LiveSessionCard } from "./LiveSessionCard";

export function LiveSessionsStrip() {
  const sessions = useSessionLive();

  if (sessions.length === 0) {
    return (
      <section style={{ marginBottom: "var(--sp-24)" }}>
        <SectionLabel>Live sessions</SectionLabel>
        <div
          style={{
            marginTop: "var(--sp-8)",
            padding: "var(--sp-16)",
            border: "var(--sp-px) dashed var(--line)",
            borderRadius: "var(--r-3)",
            color: "var(--fg-muted)",
            fontSize: "var(--fs-sm)",
            textAlign: "center",
          }}
        >
          No sessions writing right now.
        </div>
      </section>
    );
  }

  return (
    <section style={{ marginBottom: "var(--sp-24)" }}>
      <SectionLabel>
        Live sessions ({sessions.length})
      </SectionLabel>
      <div
        style={{
          marginTop: "var(--sp-8)",
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(var(--sidebar-width), 1fr))",
          gap: "var(--sp-12)",
        }}
      >
        {sessions.map((s) => (
          <LiveSessionCard key={s.session_id} summary={s} />
        ))}
      </div>
    </section>
  );
}
