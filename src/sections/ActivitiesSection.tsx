import { DashboardStrip } from "./activities/DashboardStrip";
import { SessionsSection, type SessionsSectionProps } from "./SessionsSection";

/**
 * Activities section — dashboard + cross-project session feed.
 *
 * The dashboard strip at the top answers "what's happening right now,
 * today, this month" at a glance; the session feed below (the old
 * SessionsSection) keeps the firehose + Trends / Cleanup tabs for
 * drill-down and maintenance.
 *
 * We wrap rather than absorb — SessionsSection has substantial
 * internal state (filter store, live strip, selection) that two call
 * sites sharing would entangle. The dashboard derives its own
 * aggregates from `sessionListAll` + `useSessionLive`, so the two
 * components live in parallel without coupling.
 */
export function ActivitiesSection(props: SessionsSectionProps = {}) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <DashboardStrip />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <SessionsSection {...props} />
      </div>
    </div>
  );
}
