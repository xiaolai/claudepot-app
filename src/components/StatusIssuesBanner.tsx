import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import type { StatusIssue } from "../hooks/useStatusIssues";

interface Props {
  issues: StatusIssue[];
  onDismiss: (id: string) => void;
}

/**
 * Shell-level stack of `StatusIssue` banners. Errors render first (no
 * dismiss affordance), warnings below (24 h snooze via the dismiss
 * button). Kept above the main content so drift, keychain-locked, and
 * CC-slot-drift are visible on every section, not just Accounts.
 */
export function StatusIssuesBanner({ issues, onDismiss }: Props) {
  if (issues.length === 0) return null;
  // Errors first so the first thing the user sees is the most urgent.
  const sorted = [...issues].sort((a, b) => severity(a) - severity(b));

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        padding: "var(--sp-10) var(--sp-16) 0",
      }}
      role="region"
      aria-label="Account status alerts"
    >
      {sorted.map((issue) => (
        <BannerRow
          key={issue.id}
          issue={issue}
          onDismiss={issue.dismissable ? () => onDismiss(issue.id) : undefined}
        />
      ))}
    </div>
  );
}

function severity(issue: StatusIssue): number {
  return issue.severity === "error" ? 0 : issue.severity === "warning" ? 1 : 2;
}

function BannerRow({
  issue,
  onDismiss,
}: {
  issue: StatusIssue;
  onDismiss?: () => void;
}) {
  const tone =
    issue.severity === "error"
      ? "var(--warn)"
      : issue.severity === "warning"
        ? "var(--warn)"
        : "var(--fg-muted)";
  const bg =
    issue.severity === "error"
      ? "color-mix(in oklch, var(--warn) 14%, transparent)"
      : issue.severity === "warning"
        ? "color-mix(in oklch, var(--warn) 8%, transparent)"
        : "var(--bg-sunken)";
  const glyph =
    issue.severity === "error"
      ? NF.warn
      : issue.severity === "warning"
        ? NF.warn
        : NF.info;

  return (
    <div
      role={issue.severity === "error" ? "alert" : "status"}
      style={{
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-10)",
        padding: "var(--sp-10) var(--sp-12)",
        background: bg,
        border: "var(--bw-hair) solid var(--line)",
        borderLeft: `var(--bw-accent) solid ${tone}`,
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-xs)",
      }}
    >
      <Glyph
        g={glyph}
        color={tone}
        style={{ marginTop: "var(--sp-2)", flexShrink: 0 }}
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ color: "var(--fg)", fontWeight: 600 }}>{issue.label}</div>
        {issue.detail && (
          <div style={{ color: "var(--fg-muted)", marginTop: "var(--sp-2)" }}>
            {issue.detail}
          </div>
        )}
      </div>
      {issue.action && (
        <button
          type="button"
          onClick={issue.action.onClick}
          style={{
            padding: "var(--sp-3) var(--sp-8)",
            fontSize: "var(--fs-xs)",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line-strong)",
            borderRadius: "var(--r-1)",
            color: "var(--fg)",
            cursor: "pointer",
            whiteSpace: "nowrap",
            fontWeight: 500,
            fontFamily: "inherit",
          }}
        >
          {issue.action.label}
        </button>
      )}
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss for 24 hours"
          title="Dismiss for 24 hours"
          style={{
            width: "var(--icon-btn-sm)",
            height: "var(--icon-btn-sm)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "transparent",
            color: "var(--fg-faint)",
            border: "none",
            borderRadius: "var(--r-1)",
            cursor: "pointer",
            fontFamily: "inherit",
            flexShrink: 0,
          }}
        >
          <Glyph g={NF.x} />
        </button>
      )}
    </div>
  );
}
