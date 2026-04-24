import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import type { ConfigEffectiveSettingsDto } from "../../types";

/**
 * Hooks view — first-class surface over the `hooks` field inside the
 * merged effective settings. In Claude Code, hooks aren't stand-alone
 * files; they live as arrays under a per-event key
 * (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, …) inside any
 * settings.json in the cascade. This renderer extracts, flattens, and
 * displays them with a per-row event badge and a deterministic path
 * index into the merged JSON so provenance can be looked up.
 *
 * One row per (event, matcher, command). No editing — this is a
 * read-only view of what CC would execute.
 */

interface HookRow {
  /** Event name — PreToolUse / PostToolUse / UserPromptSubmit / etc. */
  event: string;
  /** Tool matcher pattern — often a glob against tool_name. */
  matcher: string;
  /** The shell command that runs. */
  command: string;
  /** Optional timeout in ms (CC honors this). */
  timeoutMs?: number;
  /** Dotted JSON path into `merged` so the row can be sourced back to
   *  a settings.json layer via the provenance map. */
  mergedPath: string;
  /** Which scope CC would attribute this hook to (winner). Best-effort
   *  — derived from the provenance entry at `mergedPath` or its
   *  nearest ancestor. */
  winner?: string;
}

export function HooksRenderer({ cwd }: { cwd: string | null }) {
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

  const rows = useMemo<HookRow[]>(
    () => (data ? extractHookRows(data) : []),
    [data],
  );

  if (error) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--danger)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Couldn't load hooks: {error}
      </div>
    );
  }

  if (!data) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Loading…
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <div
        style={{
          padding: "var(--sp-32)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
          textAlign: "center",
        }}
      >
        No hooks registered for this project.
      </div>
    );
  }

  // Group rows by event so the view reads "what fires on PreToolUse →
  // what fires on PostToolUse …" rather than interleaved matchers.
  const byEvent = new Map<string, HookRow[]>();
  for (const r of rows) {
    const bucket = byEvent.get(r.event) ?? [];
    bucket.push(r);
    byEvent.set(r.event, bucket);
  }
  const eventOrder = Array.from(byEvent.keys()).sort();

  return (
    <div
      style={{
        flex: 1,
        overflow: "auto",
        padding: "var(--sp-16) var(--sp-20) var(--sp-24)",
      }}
    >
      {eventOrder.map((event) => (
        <section key={event} style={{ marginBottom: "var(--sp-24)" }}>
          <div
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--sp-8)",
              marginBottom: "var(--sp-8)",
            }}
          >
            <h3
              style={{
                margin: 0,
                fontSize: "var(--fs-sm)",
                fontWeight: 600,
                fontFamily: "var(--mono)",
              }}
            >
              {event}
            </h3>
            <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
              {byEvent.get(event)!.length} hook
              {byEvent.get(event)!.length === 1 ? "" : "s"}
            </span>
          </div>
          <ol
            style={{
              listStyle: "none",
              margin: 0,
              padding: 0,
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-8)",
            }}
          >
            {byEvent.get(event)!.map((row, i) => (
              <li
                key={`${row.mergedPath}#${i}`}
                style={{
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--r-2)",
                  padding: "var(--sp-10) var(--sp-12)",
                  background: "var(--bg-sunken)",
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--sp-4)",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "baseline",
                    gap: "var(--sp-8)",
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg-muted)",
                  }}
                >
                  <span style={{ fontFamily: "var(--mono)" }}>matcher</span>
                  <code
                    style={{
                      fontFamily: "var(--mono)",
                      color: "var(--fg)",
                    }}
                  >
                    {row.matcher || "(any)"}
                  </code>
                  {row.winner && (
                    <span
                      style={{
                        marginLeft: "auto",
                        fontSize: "var(--fs-2xs)",
                        color: "var(--fg-faint)",
                      }}
                      title="Winning scope for this hook entry"
                    >
                      {row.winner}
                    </span>
                  )}
                </div>
                <pre
                  style={{
                    margin: 0,
                    fontFamily: "var(--mono)",
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg)",
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                  }}
                >
                  {row.command}
                </pre>
                {row.timeoutMs != null && (
                  <div
                    style={{
                      fontSize: "var(--fs-2xs)",
                      color: "var(--fg-faint)",
                    }}
                  >
                    timeout {row.timeoutMs} ms
                  </div>
                )}
              </li>
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}

/**
 * Fast count used by the tree to decide whether to render the Hooks
 * row. Walks the same shape as `extractHookRows` but short-circuits —
 * we only care about the total number of (matcher, command) pairs.
 */
export function countHooksInMergedSettings(merged: unknown): number {
  if (!isObject(merged)) return 0;
  const hooks = (merged as Record<string, unknown>).hooks;
  if (!isObject(hooks)) return 0;
  let total = 0;
  for (const eventKey of Object.keys(hooks as object)) {
    const entries = (hooks as Record<string, unknown>)[eventKey];
    if (!Array.isArray(entries)) continue;
    for (const entry of entries) {
      if (!isObject(entry)) continue;
      const hookList = (entry as Record<string, unknown>).hooks;
      if (!Array.isArray(hookList)) continue;
      for (const h of hookList) {
        if (isObject(h)) total += 1;
      }
    }
  }
  return total;
}

/**
 * CC's hooks schema (see claudemd.ts / settings schema):
 *
 *   "hooks": {
 *     "<EventName>": [
 *       {
 *         "matcher": "<tool pattern>",
 *         "hooks": [ { "type": "command", "command": "...", "timeout_ms": 30000 }, ... ]
 *       },
 *       ...
 *     ],
 *     ...
 *   }
 *
 * Flatten one level so each (matcher, individual hook) becomes a row.
 * `mergedPath` traces back to the provenance index to let callers
 * surface "who declared this hook."
 */
function extractHookRows(data: ConfigEffectiveSettingsDto): HookRow[] {
  const out: HookRow[] = [];
  const merged = data.merged;
  if (!isObject(merged)) return out;
  const hooks = (merged as Record<string, unknown>).hooks;
  if (!isObject(hooks)) return out;

  const provIndex = buildProvIndex(data.provenance);

  for (const event of Object.keys(hooks as object)) {
    const groups = (hooks as Record<string, unknown>)[event];
    if (!Array.isArray(groups)) continue;
    groups.forEach((group, gi) => {
      if (!isObject(group)) return;
      const matcher = stringField(group, "matcher") ?? "";
      const inner = (group as Record<string, unknown>).hooks;
      if (!Array.isArray(inner)) return;
      inner.forEach((h, hi) => {
        if (!isObject(h)) return;
        const command = stringField(h, "command") ?? "";
        // Skip malformed entries with no command — they'd render as
        // blank matcher/command cards otherwise.
        if (!command.trim()) return;
        const timeoutMs =
          typeof (h as Record<string, unknown>).timeout_ms === "number"
            ? ((h as Record<string, unknown>).timeout_ms as number)
            : undefined;
        const mergedPath = `hooks.${event}[${gi}].hooks[${hi}]`;
        out.push({
          event,
          matcher,
          command,
          timeoutMs,
          mergedPath,
          winner: nearestWinner(provIndex, mergedPath),
        });
      });
    });
  }
  return out;
}

function buildProvIndex(
  prov: ConfigEffectiveSettingsDto["provenance"],
): Map<string, string> {
  const m = new Map<string, string>();
  for (const p of prov) m.set(p.path, p.winner);
  return m;
}

/**
 * Lookup path → winner, falling back to the nearest known ancestor.
 * Provenance is emitted per primitive leaf; the exact hook path won't
 * match (the whole hook group is the "leaf" in a list context), so
 * we climb to find the first hit.
 */
function nearestWinner(
  index: Map<string, string>,
  path: string,
): string | undefined {
  let cur = path;
  while (cur.length > 0) {
    const hit = index.get(cur);
    if (hit) return hit;
    // Strip one segment (either `.foo` or `[n]`).
    const lastDot = cur.lastIndexOf(".");
    const lastBracket = cur.lastIndexOf("[");
    const cut = Math.max(lastDot, lastBracket);
    if (cut <= 0) break;
    cur = cur.slice(0, cut);
  }
  return undefined;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function stringField(obj: unknown, key: string): string | undefined {
  if (!isObject(obj)) return undefined;
  const v = (obj as Record<string, unknown>)[key];
  return typeof v === "string" ? v : undefined;
}
