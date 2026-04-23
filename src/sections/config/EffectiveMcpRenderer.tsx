import { useEffect, useState } from "react";
import { api } from "../../api";
import type {
  ConfigEffectiveMcpDto,
  ConfigEffectiveMcpServerDto,
  McpSimulationMode,
} from "../../types";

const MODES: { value: McpSimulationMode; label: string; title: string }[] = [
  {
    value: "interactive",
    label: "interactive",
    title: "Default — approval prompts shown on startup.",
  },
  {
    value: "non_interactive",
    label: "non-interactive",
    title: "SDK / `claude -p` / piped input. Auto-approves project servers when projectSettings is enabled.",
  },
  {
    value: "skip_permissions",
    label: "skip-permissions",
    title: "--dangerously-skip-permissions. Auto-approves project servers when projectSettings is enabled.",
  },
];

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
      <ModePill mode={mode} onChange={setMode} />
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

function ModePill({
  mode,
  onChange,
}: {
  mode: McpSimulationMode;
  onChange: (m: McpSimulationMode) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label="MCP simulation mode"
      style={{
        display: "flex",
        gap: "var(--sp-4)",
        padding: "var(--sp-8) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        alignItems: "center",
      }}
    >
      <span
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          marginRight: "var(--sp-4)",
        }}
      >
        simulate:
      </span>
      {MODES.map((m) => {
        const active = m.value === mode;
        return (
          <button
            key={m.value}
            type="button"
            role="radio"
            aria-checked={active}
            title={m.title}
            onClick={() => onChange(m.value)}
            className="pm-focus"
            style={{
              padding: "var(--sp-3) var(--sp-10)",
              fontSize: "var(--fs-2xs)",
              fontFamily: "var(--mono)",
              background: active ? "var(--accent-soft)" : "transparent",
              color: active ? "var(--accent-ink)" : "var(--fg-muted)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              cursor: "pointer",
            }}
          >
            {m.label}
          </button>
        );
      })}
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
            Approval
          </th>
          <th style={{ padding: "var(--sp-6) var(--sp-12)", fontWeight: 500 }}>
            Command
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
        <td style={{ padding: "var(--sp-6) var(--sp-12)" }}>
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
            fontFamily: "var(--mono)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
          }}
        >
          {server.source_scope}
        </td>
        <td style={{ padding: "var(--sp-6) var(--sp-12)" }}>
          <ApprovalBadge
            approval={server.approval}
            reason={server.approval_reason}
            blockedBy={server.blocked_by}
          />
        </td>
        <td
          style={{
            padding: "var(--sp-6) var(--sp-12)",
            fontFamily: "var(--mono)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            maxWidth: 240,
          }}
          title={cmd}
        >
          {cmd}
        </td>
      </tr>
      {open && (
        <tr>
          <td colSpan={4} style={{ background: "var(--bg-sunken)" }}>
            <pre
              style={{
                margin: 0,
                padding: "var(--sp-10) var(--sp-16)",
                fontFamily: "var(--mono)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-muted)",
                whiteSpace: "pre-wrap",
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
  const colorMap: Record<typeof approval, { bg: string; fg: string }> = {
    approved: { bg: "var(--accent-soft)", fg: "var(--accent-ink)" },
    auto_approved: { bg: "var(--accent-soft)", fg: "var(--accent-ink)" },
    pending: { bg: "var(--bg-sunken)", fg: "var(--fg-muted)" },
    rejected: { bg: "var(--bg-sunken)", fg: "var(--danger)" },
  };
  const c = colorMap[approval];
  const label = approval.replace(/_/g, " ");
  const title =
    blockedBy != null
      ? `blocked: ${blockedBy}`
      : reason != null
        ? `reason: ${reason}`
        : undefined;
  return (
    <span
      title={title}
      style={{
        display: "inline-block",
        padding: "0 var(--sp-6)",
        fontSize: "var(--fs-2xs)",
        fontFamily: "var(--mono)",
        color: c.fg,
        background: c.bg,
        borderRadius: "var(--r-1)",
        border: "var(--bw-hair) solid var(--line)",
      }}
    >
      {label}
    </span>
  );
}
