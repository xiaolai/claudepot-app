import { useEffect, useState } from "react";
import { api } from "../../api";
import type {
  ConfigEffectiveMcpDto,
  ConfigEffectiveMcpServerDto,
  McpSimulationMode,
} from "../../types";
import { Tag, type TagTone } from "../../components/primitives/Tag";
import { SegmentedControl } from "../../components/SegmentedControl";

// Display labels for the simulation segmented control. Kept short so
// all three fit at any reasonable pane width (the longest, `non-int`,
// matches `skip-perm` for visual balance). Hover surfaces the full
// command-line equivalent via `MODE_TITLES`.
const MODES: readonly { id: McpSimulationMode; label: string }[] = [
  { id: "interactive", label: "interactive" },
  { id: "non_interactive", label: "non-interactive" },
  { id: "skip_permissions", label: "skip-perms" },
] as const;

const MODE_TITLES: Record<McpSimulationMode, string> = {
  interactive: "Default — approval prompts shown on startup.",
  non_interactive:
    "SDK / `claude -p` / piped input. Auto-approves project servers when projectSettings is enabled.",
  skip_permissions:
    "--dangerously-skip-permissions. Auto-approves project servers when projectSettings is enabled.",
};

/**
 * Effective MCP view — shows every MCP server CC would consider, the
 * scope that contributed it, and the approval state CC would produce
 * in the chosen simulation mode (plan §9.3 / D17).
 *
 * Simulation mode pill is local state — not persisted to CC. Changing
 * it re-requests the server list.
 */
export function EffectiveMcpRenderer({ cwd }: { cwd: string | null }) {
  const [mode, setMode] = useState<McpSimulationMode>("interactive");
  const [data, setData] = useState<ConfigEffectiveMcpDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    setData(null);
    void api
      .configEffectiveMcp(mode, cwd)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [mode, cwd]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        flex: 1,
        minHeight: 0,
      }}
    >
      <ModeBar mode={mode} onChange={setMode} />
      {data?.enterprise_lockout && <EnterpriseBanner />}
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {error ? (
          <div
            style={{
              padding: "var(--sp-20)",
              color: "var(--danger)",
              fontSize: "var(--fs-sm)",
            }}
          >
            Couldn't compute effective MCP: {error}
          </div>
        ) : !data ? (
          <div style={{ padding: "var(--sp-20)", color: "var(--fg-faint)" }}>
            Loading…
          </div>
        ) : data.servers.length === 0 ? (
          <div
            style={{
              padding: "var(--sp-28)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            No MCP servers configured at any scope.
          </div>
        ) : (
          <ServerTable servers={data.servers} />
        )}
      </div>
    </div>
  );
}

function ModeBar({
  mode,
  onChange,
}: {
  mode: McpSimulationMode;
  onChange: (m: McpSimulationMode) => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        padding: "var(--sp-8) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
      title={MODE_TITLES[mode]}
    >
      <span
        className="mono-cap"
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        Simulate
      </span>
      <SegmentedControl options={MODES} value={mode} onChange={onChange} />
    </div>
  );
}

function EnterpriseBanner() {
  return (
    <div
      role="status"
      style={{
        padding: "var(--sp-8) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        color: "var(--fg)",
        fontSize: "var(--fs-xs)",
      }}
    >
      <strong>Enterprise policy in effect.</strong> User / project / local
      MCP servers are suppressed — only enterprise servers are active.
    </div>
  );
}

function ServerTable({
  servers,
}: {
  servers: ConfigEffectiveMcpServerDto[];
}) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-xs)",
        // Auto layout — atomic columns (Source: "user"/"project"/…,
        // Approval: a tag) hug their content; the variable-length
        // columns (Server, Command) split the remainder. Cells in the
        // variable columns set `max-width: 0` + `text-overflow:
        // ellipsis` so they truncate instead of forcing the table
        // wider than its container. Earlier attempts at proportional
        // `tableLayout: fixed` either truncated atomic columns to
        // useless widths ("proje…") or, when mixed with px values,
        // computed a negative width for Server in narrow panes.
        tableLayout: "auto",
      }}
    >
      <thead>
        <tr
          style={{
            textAlign: "left",
            color: "var(--fg-faint)",
            borderBottom: "var(--bw-hair) solid var(--line)",
          }}
        >
          <th style={{ padding: "var(--sp-6) var(--sp-12)", fontWeight: 500 }}>
            Server
          </th>
          <th style={{ padding: "var(--sp-6) var(--sp-12)", fontWeight: 500 }}>
            Source
          </th>
          <th style={{ padding: "var(--sp-6) var(--sp-12)", fontWeight: 500 }}>
            Command
          </th>
          <th style={{ padding: "var(--sp-6) var(--sp-12)", fontWeight: 500 }}>
            Approval
          </th>
        </tr>
      </thead>
      <tbody>
        {servers.map((s) => (
          <ServerRow key={s.name} server={s} />
        ))}
      </tbody>
    </table>
  );
}

function ServerRow({ server }: { server: ConfigEffectiveMcpServerDto }) {
  const [open, setOpen] = useState(false);
  const cmd =
    (server.masked as { command?: string } | null)?.command ?? "";
  return (
    <>
      <tr
        style={{
          borderBottom: "var(--bw-hair) solid var(--line)",
          cursor: "pointer",
        }}
        onClick={() => setOpen((v) => !v)}
      >
        <td
          style={{
            // `max-width: 0` + `width: 100%` is the table-cell trick
            // that lets `text-overflow: ellipsis` actually fire under
            // `table-layout: auto` — without it, the cell expands to
            // its content's intrinsic width and pushes the table past
            // its container. The auto layout then redistributes the
            // remainder between this column and Command.
            maxWidth: 0,
            width: "100%",
            padding: "var(--sp-6) var(--sp-12)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={server.name}
        >
          <span style={{ fontWeight: 500 }}>{server.name}</span>
          {server.contributors.length > 1 && (
            <span
              style={{
                marginLeft: "var(--sp-6)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
              title={server.contributors.join(", ")}
            >
              +{server.contributors.length - 1}
            </span>
          )}
        </td>
        <td
          style={{
            padding: "var(--sp-6) var(--sp-12)",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={server.source_scope}
        >
          {server.source_scope}
        </td>
        <td
          style={{
            // Same `max-width: 0` + `width: 100%` ellipsis trick as
            // the Server cell — the two variable-length columns share
            // whatever the atomic Source/Approval cells leave behind.
            maxWidth: 0,
            width: "100%",
            padding: "var(--sp-6) var(--sp-12)",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={cmd}
        >
          {cmd}
        </td>
        <td style={{ padding: "var(--sp-6) var(--sp-12)" }}>
          <ApprovalBadge
            approval={server.approval}
            reason={server.approval_reason}
            blockedBy={server.blocked_by}
          />
        </td>
      </tr>
      {open && (
        <tr>
          <td colSpan={4} style={{ background: "var(--bg-sunken)" }}>
            <pre
              style={{
                margin: 0,
                padding: "var(--sp-10) var(--sp-16)",
                fontFamily: "var(--font-mono)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-muted)",
                whiteSpace: "pre-wrap",
                overflowWrap: "anywhere",
              }}
            >
              {JSON.stringify(server.masked, null, 2)}
            </pre>
          </td>
        </tr>
      )}
    </>
  );
}

function ApprovalBadge({
  approval,
  reason,
  blockedBy,
}: {
  approval: ConfigEffectiveMcpServerDto["approval"];
  reason: string | null;
  blockedBy: string | null;
}) {
  const tone: TagTone =
    approval === "approved"
      ? "ok"
      : approval === "auto_approved"
        ? "accent"
        : approval === "rejected"
          ? "danger"
          : "neutral";
  const label = approval.replace(/_/g, " ");
  const title =
    blockedBy != null
      ? `blocked: ${blockedBy}`
      : reason != null
        ? `reason: ${reason}`
        : undefined;
  return (
    <Tag tone={tone} title={title}>
      {label}
    </Tag>
  );
}
