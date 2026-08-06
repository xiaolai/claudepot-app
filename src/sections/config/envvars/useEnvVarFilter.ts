// Search-and-filter state for the pane.
//
// Extracted so the pane component is about orchestration — load, write,
// confirm — and this is about which rows survive. The filter is where the
// "search first" decision actually lives: name AND doc text, because a user
// typing "telemetry" or "milliseconds" is searching the prose, not the
// identifier.

import { useMemo, useState } from "react";
import type { EnvControl, EnvOverview, EnvVarState } from "../../../types/ccEnv";

export type SafetyFilter = "secret" | "hazard" | "provider" | "set";

export const CONTROLS: EnvControl[] = ["toggle", "enum", "number", "text"];

/** Catalog keys, not literals — the chip label is looked up at render
 *  time so a language switch reaches a toolbar already on screen. */
export const SAFETY_FILTERS = [
  { key: "set", labelKey: "envvars.filterModified" },
  { key: "secret", labelKey: "envvars.filterSecret" },
  { key: "hazard", labelKey: "envvars.filterRisky" },
  { key: "provider", labelKey: "envvars.filterProviderManaged" },
] as const satisfies readonly { key: SafetyFilter; labelKey: string }[];

function toggleIn<T>(set: Set<T>, v: T): Set<T> {
  const next = new Set(set);
  if (!next.delete(v)) next.add(v);
  return next;
}

export function useEnvVarFilter(data: EnvOverview | null) {
  const [query, setQuery] = useState("");
  const [controls, setControls] = useState<Set<EnvControl>>(new Set());
  const [safety, setSafety] = useState<Set<SafetyFilter>>(new Set());

  const rows: EnvVarState[] = useMemo(() => {
    if (!data) return [];
    const q = query.trim().toLowerCase();
    return data.documented.filter((v) => {
      if (controls.size && !controls.has(v.spec.control)) return false;
      if (safety.has("secret") && !v.spec.safety.secret) return false;
      if (
        safety.has("hazard") &&
        !v.spec.safety.hazards.some((h) => h !== "unknown")
      ) {
        return false;
      }
      if (safety.has("provider") && !v.spec.safety.provider_managed) {
        return false;
      }
      if (safety.has("set") && v.settings_value.state === "absent") return false;
      if (!q) return true;
      return (
        v.spec.name.toLowerCase().includes(q) ||
        v.spec.doc.toLowerCase().includes(q)
      );
    });
  }, [controls, data, query, safety]);

  return {
    query,
    setQuery,
    controls,
    safety,
    rows,
    toggleControl: (c: EnvControl) => setControls((s) => toggleIn(s, c)),
    toggleSafety: (f: SafetyFilter) => setSafety((s) => toggleIn(s, f)),
  };
}
