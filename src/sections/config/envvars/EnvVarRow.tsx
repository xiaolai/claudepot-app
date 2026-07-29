// One documented variable: name, safety markers, chooser, help text.
//
// Rows are variable-height by design — help and warning text are the
// point, not decoration — which is why the list measures dynamically
// rather than assuming the Config tree's fixed row height.

import { Glyph } from "../../../components/primitives";
import { NF } from "../../../icons";
import type { EnvVarState } from "../../../types/ccEnv";
import { EnvVarControl, type ControlHandlers } from "./EnvVarControl";
import { hasUnnamedHazard, HAZARD_WARNING, namedHazards } from "./hazards";

const BLOCKED_TEXT = {
  bootstrap_split_brain:
    "Claude Code reads this before it loads settings, so a value here would leave one run resolving paths against two different directories. Set it in your shell instead.",
  host_injected:
    "Injected per run by whatever launches Claude Code. A value written here is overwritten immediately.",
} as const;

export function EnvVarRow({
  state,
  busy,
  handlers,
}: {
  state: EnvVarState;
  busy: boolean;
  handlers: ControlHandlers;
}) {
  const { spec, settings_value: value, legacy_global, resolved_source } = state;
  const blocked = spec.safety.blocked_reason;
  const hazards = namedHazards(spec.safety.hazards);
  const unestablished = hasUnnamedHazard(spec.safety.hazards);
  const isSet = value.state !== "absent";

  return (
    <div className="envvar-row" data-set={isSet ? "true" : "false"}>
      <div className="envvar-row-head">
        <code className="envvar-name selectable">{spec.name}</code>
        <div className="envvar-tags">
          {isSet ? (
            <span className="envvar-badge" data-tone="set">
              modified
            </span>
          ) : null}
          {spec.safety.secret ? (
            <span className="envvar-badge" data-tone="secret">
              secret
            </span>
          ) : null}
          {spec.safety.provider_managed ? (
            <span className="envvar-badge" data-tone="muted">
              provider-managed
            </span>
          ) : null}
          <span className="envvar-badge" data-tone="muted">
            {spec.control}
          </span>
        </div>
        <div className="envvar-control">
          <EnvVarControl
            spec={spec}
            value={value}
            busy={busy}
            handlers={handlers}
          />
        </div>
      </div>

      <p className="envvar-doc">{spec.doc}</p>

      {/* Required disclosure sits inline next to the control, never in a
          tooltip — rules/design.md and rules/icon-buttons.md both treat
          tooltip-as-required-disclosure as an anti-pattern. */}
      {blocked ? (
        <p className="envvar-note" data-tone="warn">
          <Glyph g={NF.lock} /> {BLOCKED_TEXT[blocked]}
        </p>
      ) : null}
      {hazards.map((h) => (
        <p key={h} className="envvar-note" data-tone="warn">
          <Glyph g={NF.warn} /> {HAZARD_WARNING[h]}
        </p>
      ))}
      {/* Absence from Claude Code's allowlist says something is risky
          without saying what. Naming a specific risk here would be the
          same sin as guessing a control type, so this is muted, and
          visibly different from the named hazards above. */}
      {unestablished && hazards.length === 0 && !blocked ? (
        <p className="envvar-note" data-tone="muted">
          Not on Claude Code's pre-trust allowlist. No specific risk is
          documented for it.
        </p>
      ) : null}
      {spec.safety.provider_managed ? (
        <p className="envvar-note" data-tone="muted">
          A host-managed launch can override this regardless of what is set
          here.
        </p>
      ) : null}
      {resolved_source === "legacy_global" && legacy_global ? (
        <p className="envvar-note" data-tone="muted">
          Also set in <code>~/.claude.json</code>, which Claude Code applies
          first. With no settings.json override, that value is what takes
          effect.
        </p>
      ) : null}
      {resolved_source === "settings_override" && legacy_global ? (
        <p className="envvar-note" data-tone="muted">
          <code>~/.claude.json</code> also sets this. Clearing here lets that
          value surface.
        </p>
      ) : null}
      {!isSet ? (
        <p className="envvar-note" data-tone="muted">
          No settings.json override
          {spec.default ? ` — documented default ${spec.default}` : ""}.
        </p>
      ) : null}
    </div>
  );
}
