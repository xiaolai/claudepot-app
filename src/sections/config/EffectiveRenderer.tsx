import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import type { ConfigEffectiveSettingsDto } from "../../types";

/**
 * Merged settings renderer. Walks the merged JSON, emits one row per
 * primitive leaf, badges each with its winning scope. Hover a row to
 * see the contributor list — aggregated from the flattened
 * ProvenanceEntry list by prefix match, per plan §8.5.
 *
 * Design: paper-mono. One primary action (reveal origins via hover /
 * focus). Suppressed leaves get a subtle strikethrough so users can
 * see which containers got clobbered by a higher-priority null/scalar.
 */
export function EffectiveRenderer({ cwd }: { cwd: string | null }) {
  const [data, setData] = useState<ConfigEffectiveSettingsDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    setData(null);
    void api
      .configEffectiveSettings(cwd)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [cwd]);

  const provByPath = useMemo(() => {
    const m = new Map<
      string,
      { winner: string; contributors: string[]; suppressed: boolean }
    >();
    for (const p of data?.provenance ?? []) {
      m.set(p.path, {
        winner: p.winner,
        contributors: p.contributors,
        suppressed: p.suppressed,
      });
    }
    return m;
  }, [data]);

  if (error) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--danger)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Couldn't compute effective settings: {error}
      </div>
    );
  }
  if (!data) {
    return (
      <div style={{ padding: "var(--sp-20)", color: "var(--fg-faint)" }}>
        Computing…
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        minWidth: 0,
        flex: 1,
      }}
    >
      <PolicyBanner
        winner={data.policy_winner}
        errors={data.policy_errors}
      />
      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-12) var(--sp-16)",
          fontFamily: "var(--font-mono)",
          fontSize: "var(--fs-xs)",
        }}
      >
        <TreeView value={data.merged} prov={provByPath} />
      </div>
    </div>
  );
}

function PolicyBanner({
  winner,
  errors,
}: {
  winner: string | null;
  errors: ConfigEffectiveSettingsDto["policy_errors"];
}) {
  if (!winner && errors.length === 0) return null;
  return (
    <div
      style={{
        padding: "var(--sp-8) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
      }}
    >
      {winner && (
        <div>
          <strong>Policy active:</strong> {winner}
        </div>
      )}
      {errors.length > 0 && (
        <div style={{ marginTop: "var(--sp-4)" }}>
          {errors.length} rejected source
          {errors.length === 1 ? "" : "s"}:{" "}
          {errors.map((e) => `${e.origin} (${e.message})`).join("; ")}
        </div>
      )}
    </div>
  );
}

function TreeView({
  value,
  prov,
  path = "",
  depth = 0,
}: {
  value: unknown;
  prov: Map<string, {
    winner: string;
    contributors: string[];
    suppressed: boolean;
  }>;
  path?: string;
  depth?: number;
}) {
  if (value === null || typeof value !== "object") {
    return <Leaf path={path} value={value} prov={prov} />;
  }
  if (Array.isArray(value)) {
    return (
      <div>
        <Bracket>[</Bracket>
        <div style={{ marginLeft: "var(--sp-16)" }}>
          {value.map((v, i) => {
            const childPath = `${path}[${i}]`;
            return (
              <div key={i}>
                <TreeView
                  value={v}
                  prov={prov}
                  path={childPath}
                  depth={depth + 1}
                />
                {i < value.length - 1 && <Punct>,</Punct>}
              </div>
            );
          })}
        </div>
        <Bracket>]</Bracket>
      </div>
    );
  }
  const obj = value as Record<string, unknown>;
  const keys = Object.keys(obj);
  return (
    <div>
      <Bracket>{"{"}</Bracket>
      <div style={{ marginLeft: "var(--sp-16)" }}>
        {keys.map((k, i) => {
          const childPath = path ? `${path}.${k}` : k;
          return (
            <div key={k}>
              <Key>{JSON.stringify(k)}</Key>
              <Punct>: </Punct>
              <TreeView
                value={obj[k]}
                prov={prov}
                path={childPath}
                depth={depth + 1}
              />
              {i < keys.length - 1 && <Punct>,</Punct>}
            </div>
          );
        })}
      </div>
      <Bracket>{"}"}</Bracket>
    </div>
  );
}

function Leaf({
  path,
  value,
  prov,
}: {
  path: string;
  value: unknown;
  prov: Map<string, {
    winner: string;
    contributors: string[];
    suppressed: boolean;
  }>;
}) {
  const p = prov.get(path);
  const winner = p?.winner;
  const suppressed = !!p?.suppressed;
  const contribList = (p?.contributors ?? [])
    .filter((c) => c !== winner)
    .join(", ");
  const title = winner
    ? `winner: ${winner}${contribList ? `; also contributors: ${contribList}` : ""}${
        suppressed ? " — suppressed" : ""
      }`
    : undefined;
  return (
    <span
      title={title}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
      }}
    >
      <Value value={value} suppressed={suppressed} />
      {winner && <ScopeBadge label={winner} />}
    </span>
  );
}

function Value({ value, suppressed }: { value: unknown; suppressed: boolean }) {
  const text =
    value === null
      ? "null"
      : typeof value === "string"
        ? JSON.stringify(value)
        : String(value);
  const color =
    typeof value === "string"
      ? "var(--accent-ink)"
      : typeof value === "number" || typeof value === "boolean"
        ? "var(--fg)"
        : "var(--fg-muted)";
  return (
    <span
      style={{
        color,
        textDecoration: suppressed ? "line-through" : "none",
      }}
    >
      {text}
    </span>
  );
}

function ScopeBadge({ label }: { label: string }) {
  // Lightweight mono badge — Tag is too heavy for a dense inline
  // annotation on every primitive leaf. Keeps visual noise low.
  return (
    <span
      style={{
        fontSize: "var(--fs-2xs)",
        padding: "0 var(--sp-4)",
        background: "var(--bg-sunken)",
        color: "var(--fg-muted)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        fontFamily: "var(--font-mono)",
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </span>
  );
}

function Bracket({ children }: { children: string }) {
  return <span style={{ color: "var(--fg-muted)" }}>{children}</span>;
}
function Punct({ children }: { children: string }) {
  return <span style={{ color: "var(--fg-faint)" }}>{children}</span>;
}
function Key({ children }: { children: string }) {
  return <span style={{ color: "var(--fg)" }}>{children}</span>;
}
