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
      <section style={{ marginBottom: 24 }}>
        <SectionLabel>Live sessions</SectionLabel>
        <div
          style={{
            marginTop: 8,
            padding: 16,
            border: "tokens.sp.px dashed var(--line)",
            borderRadius: 8,
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
    <section style={{ marginBottom: 24 }}>
      <SectionLabel>
        Live sessions ({sessions.length})
      </SectionLabel>
      <div
        style={{
          marginTop: 8,
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(tokens.sidebar.width, 1fr))",
          gap: 12,
        }}
      >
        {sessions.map((s) => (
          <LiveSessionCard key={s.session_id} summary={s} />
        ))}
      </div>
    </section>
  );
}
