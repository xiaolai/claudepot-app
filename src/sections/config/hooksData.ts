// Pure data extraction for hooks rows. Sharded out of HooksRenderer
// so the renderer file stays under the loc-guardian limit and so
// ConfigSection can import the count helper without dragging in the
// full renderer module.

import type { ConfigEffectiveSettingsDto } from "../../types";

export interface HookRow {
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
export function extractHookRows(data: ConfigEffectiveSettingsDto): HookRow[] {
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
