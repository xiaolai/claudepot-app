import { Glyph } from "../components/primitives/Glyph";
import { useDaemonStatus } from "../hooks/useDaemonStatus";
import { NF } from "../icons";

/**
 * Small render-if-nonzero indicator showing the number of active
 * background CC workers held by the per-user daemon. Sits between
 * the live Activity strip and the sync/collapse strip so it reads
 * as "live state" rather than chrome.
 *
 * Hidden when the daemon is idle or the scrape couldn't pin down a
 * count — a zero is no signal worth the row. See
 * `dev-docs/cc-daemon-research.md` for the broader context.
 */
export function SidebarBgBadge({ collapsed }: { collapsed?: boolean }) {
  const { status } = useDaemonStatus();
  const workers = status?.running ? status.bgWorkers ?? 0 : 0;
  if (workers <= 0) return null;

  const label = `${workers} bg worker${workers === 1 ? "" : "s"}`;

  if (collapsed) {
    // Icon + count, vertically centered. Tooltip carries the full
    // phrase since the surrounding column is icon-only at this width.
    return (
      <div
        title={label}
        aria-label={label}
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: "var(--sp-2)",
          padding: "var(--sp-6) 0",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-2xs)",
        }}
      >
        <Glyph g={NF.cpu} />
        <span>{workers}</span>
      </div>
    );
  }

  return (
    <div
      style={{
        padding: "var(--sp-6) var(--sp-14)",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
      }}
    >
      <Glyph g={NF.cpu} />
      <span>{label}</span>
    </div>
  );
}
