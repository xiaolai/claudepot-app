import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";

/**
 * Placeholder for the Sessions screen. The backend doesn't yet
 * expose a "list all sessions" surface (only orphan detection), so
 * this view reserves the route and tells the user where to look in
 * the meantime. Fleshed out in a future phase once the backend
 * surfaces `session_list` / `session_show`.
 */
export function SessionsSection() {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        flex: 1,
        minHeight: 0,
      }}
    >
      <ScreenHeader
        crumbs={["claudepot", "sessions"]}
        title="Sessions"
        subtitle="Recent Claude Code conversations by project"
      />
      <div
        style={{
          flex: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "var(--sp-48)",
        }}
      >
        <div
          role="status"
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--sp-8)",
            maxWidth: "var(--content-cap-sm)",
            textAlign: "center",
            color: "var(--fg-muted)",
          }}
        >
          <Glyph g={NF.chatAlt} size="var(--sp-32)" color="var(--fg-ghost)" />
          <p style={{ margin: 0, fontSize: "var(--fs-base)" }}>
            Sessions view coming soon.
          </p>
          <p
            style={{
              margin: 0,
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
            }}
          >
            Session transcripts live under{" "}
            <code style={{ fontFamily: "var(--font)" }}>
              ~/.claude/projects/&lt;slug&gt;/
            </code>
            . Open the Projects screen to navigate to a project and
            inspect its sessions.
          </p>
        </div>
      </div>
    </div>
  );
}
