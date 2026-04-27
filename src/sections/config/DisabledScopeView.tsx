// Right-pane view for the Config tree's "Disabled" virtual scope.
// Orchestrates two columns:
//   - Left:  DisabledList (grouped by kind + scope_root, with
//            Re-enable / Trash per row)
//   - Right: DisabledPreview (read via artifact_disabled_preview)
// Both children live in `./disabled/` so each shard stays under the
// loc-guardian limit.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { DisabledRecordDto } from "../../types";
import { DisabledList, matches } from "./disabled/DisabledList";
import { DisabledPreview } from "./disabled/DisabledPreview";

export function DisabledScopeView({
  projectRoot,
  pushToast,
  onChanged,
}: {
  /** Active project anchor's `.claude/` dir, or null in global mode. */
  projectRoot: string | null;
  pushToast: (kind: "info" | "error", text: string) => void;
  /** Bumped after re-enable so the parent can refresh the active tree. */
  onChanged: () => void;
}) {
  const [rows, setRows] = useState<DisabledRecordDto[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [selected, setSelected] = useState<DisabledRecordDto | null>(null);

  const refresh = useCallback(() => {
    setRefreshTick((n) => n + 1);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    api
      .artifactListDisabled(projectRoot)
      .then((d) => {
        if (cancelled) return;
        setRows(d);
        // Clear selection when the row goes away (e.g., after enable).
        setSelected((cur) => (cur && d.some(matches(cur)) ? cur : null));
      })
      .catch((err) => {
        if (cancelled) return;
        setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [projectRoot, refreshTick]);

  if (loadError) {
    return (
      <Pane>
        <Empty danger>Couldn't load disabled artifacts: {loadError}</Empty>
      </Pane>
    );
  }
  if (rows === null) {
    return (
      <Pane>
        <Empty>Loading…</Empty>
      </Pane>
    );
  }
  if (rows.length === 0) {
    return (
      <Pane>
        <Empty>
          Nothing is disabled. Use the <em>Disable</em> action in the Config
          tree to hide a Skill, Agent, or Slash command from Claude Code
          without deleting it. Disabled artifacts can be re-enabled in one
          click and stay on disk under <code>.disabled/</code> until you
          trash them.
        </Empty>
      </Pane>
    );
  }

  return (
    <Pane>
      <Header count={rows.length} />
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(0, 320px) minmax(0, 1fr)",
          gap: "var(--sp-16)",
          flex: 1,
          minHeight: 0,
        }}
      >
        <DisabledList
          rows={rows}
          selected={selected}
          onSelect={setSelected}
          projectRoot={projectRoot}
          pushToast={pushToast}
          onChanged={() => {
            refresh();
            onChanged();
          }}
        />
        <DisabledPreview row={selected} projectRoot={projectRoot} />
      </div>
    </Pane>
  );
}

function Pane({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        padding: "var(--sp-16) var(--sp-20) var(--sp-24)",
        gap: "var(--sp-16)",
      }}
    >
      {children}
    </div>
  );
}

function Header({ count }: { count: number }) {
  return (
    <header
      style={{
        display: "flex",
        alignItems: "baseline",
        gap: "var(--sp-12)",
      }}
    >
      <h2
        style={{
          margin: 0,
          fontSize: "var(--fs-base)",
          fontWeight: 600,
          color: "var(--fg)",
        }}
      >
        Disabled artifacts
      </h2>
      <span
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        {count} hidden from Claude Code
      </span>
    </header>
  );
}

function Empty({
  children,
  danger,
}: {
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-20) var(--sp-24)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-sm)",
        color: danger ? "var(--danger)" : "var(--fg-muted)",
        maxWidth: "60ch",
      }}
    >
      {children}
    </div>
  );
}
