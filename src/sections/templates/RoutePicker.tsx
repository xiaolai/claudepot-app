import { useEffect } from "react";
import type { TemplateRouteSummaryDto } from "../../types";

interface Props {
  routes: TemplateRouteSummaryDto[];
  selectedRouteId: string | null;
  onChange: (routeId: string | null) => void;
  /** When the template requires `local-only` and the user has
   *  no local route, render a deep-link instead of the picker. */
  privacyClass: string;
  onOpenThirdParties: () => void;
}

/**
 * Project the user's Third-parties tab into a capability-filtered
 * dropdown. Renders nothing when:
 *
 * - 0 capable routes exist (template runs on default `claude`)
 * - 1 capable route exists (silently used)
 *
 * Renders a dropdown when N>1. For `local-only` templates with
 * zero local routes, renders a deep-link to the Third-parties
 * tab — no inline setup wizard.
 */
export function RoutePicker({
  routes,
  selectedRouteId,
  onChange,
  privacyClass,
  onOpenThirdParties,
}: Props) {
  const capable = routes.filter((r) => r.is_capable);
  const ineligible = routes.filter((r) => !r.is_capable);

  // Auto-select the only capable route when there's exactly one.
  // Side effects must live in useEffect — calling onChange via
  // queueMicrotask during render scheduled a state update on
  // every render, causing a visible re-render flash on first
  // mount.
  useEffect(() => {
    if (
      capable.length === 1 &&
      ineligible.length === 0 &&
      selectedRouteId !== capable[0].id
    ) {
      onChange(capable[0].id);
    }
    // We intentionally depend on the route ids only — full
    // route objects re-allocate every render, but their ids
    // don't.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [capable.map((r) => r.id).join(","), ineligible.length, selectedRouteId]);

  if (privacyClass === "local" && capable.length === 0) {
    return (
      <div
        style={{
          padding: "var(--sp-8) var(--sp-12)",
          border: "var(--bw-hair) solid var(--accent)",
          borderRadius: "var(--r-2)",
          background: "var(--bg-sunken)",
          color: "var(--fg)",
          fontSize: "var(--fs-sm)",
        }}
      >
        This template runs only on a local route, but you don&rsquo;t have one
        configured.{" "}
        <button
          type="button"
          onClick={onOpenThirdParties}
          style={{
            background: "none",
            border: "none",
            color: "var(--accent)",
            textDecoration: "underline",
            cursor: "pointer",
            font: "inherit",
            padding: 0,
          }}
        >
          Set one up in Third-parties.
        </button>
      </div>
    );
  }

  // 0 routes → nothing rendered; the install proceeds against default `claude`.
  // 1 capable route → silently used (auto-select via the effect above).
  if (capable.length === 0) return null;
  if (capable.length === 1 && ineligible.length === 0) return null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      <span
        className="mono-cap"
        style={{
          color: "var(--fg-faint)",
          fontSize: "var(--fs-2xs)",
        }}
      >
        Run with
      </span>
      <select
        value={selectedRouteId ?? "__default__"}
        onChange={(e) => {
          const v = e.target.value;
          onChange(v === "__default__" ? null : v);
        }}
        style={{
          padding: "var(--sp-6) var(--sp-8)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          background: "var(--bg-raised)",
          color: "var(--fg)",
          fontSize: "var(--fs-sm)",
        }}
      >
        <option value="__default__">Default — claude (your CLI account)</option>
        {capable.map((r) => (
          <option key={r.id} value={r.id}>
            {r.name} · {r.model}
            {r.is_local ? " · local" : ""}
          </option>
        ))}
        {ineligible.length > 0 && (
          <optgroup label="Not eligible">
            {ineligible.map((r) => (
              <option key={r.id} value={r.id} disabled>
                {r.name} · {r.model} — {r.ineligibility_reason}
              </option>
            ))}
          </optgroup>
        )}
      </select>
    </div>
  );
}
