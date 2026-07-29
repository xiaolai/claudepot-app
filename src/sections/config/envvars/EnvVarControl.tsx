// The editing affordance for one variable — everything to the right of
// the name. Split from the row so that the row is about *what to say*
// about a variable and this is about *what you can do to it*.

import { Button } from "../../../components/primitives";
import type { EnvValue, EnvVarSpec } from "../../../types/ccEnv";
import {
  commitModeFor,
  EnumChooser,
  ScalarChooser,
  SecretChooser,
  ToggleTriState,
  ToggleTwoState,
  UNSET,
  type SecretHandle,
  type TriState,
} from "./choosers";

export interface ControlHandlers {
  onSet: (name: string, value: string) => void;
  onClear: (name: string) => void;
  onRequestSecret: (name: string, handle: SecretHandle) => void;
}

function ClearButton({
  name,
  busy,
  onClear,
}: {
  name: string;
  busy: boolean;
  onClear: (name: string) => void;
}) {
  return (
    <Button variant="ghost" disabled={busy} onClick={() => onClear(name)}>
      Clear
    </Button>
  );
}

/**
 * A value the chooser cannot round-trip.
 *
 * Rendered read-only with its raw value and two explicit actions, because
 * feeding it to the chooser would coerce it and the next interaction would
 * write the coerced form over the user's data.
 *
 * **Replace clears the key; it does not write the documented default.** An
 * earlier version sent `spec.default || ""`, which broke twice over: it
 * violated the rule that documented defaults are display-only, and for the
 * many specs with no documented default it sent `""` — a value the backend
 * rejects for a number or an enum. Clearing hands the row back to its normal
 * chooser with nothing pre-decided, which is what "replace" meant.
 */
function CustomValueControl({
  spec,
  value,
  busy,
  onClear,
}: {
  spec: EnvVarSpec;
  value: Extract<EnvValue, { state: "custom" } | { state: "custom_opaque" }>;
  busy: boolean;
  onClear: (name: string) => void;
}) {
  return (
    <span className="envvar-scalar">
      {value.state === "custom" ? (
        // An explicit `""` is the third state this pane exists to preserve.
        // Rendering it as an empty node would make it indistinguishable
        // from unset — which is the confusion, not a display detail.
        <code className="envvar-custom-raw selectable">
          {value.raw === "" ? "(empty string)" : value.raw}
        </code>
      ) : (
        <span className="envvar-custom-raw">
          a JSON {value.kind} — contents not shown
        </span>
      )}
      <Button variant="subtle" disabled={busy} onClick={() => onClear(spec.name)}>
        Replace
      </Button>
      <ClearButton name={spec.name} busy={busy} onClear={onClear} />
    </span>
  );
}

export function EnvVarControl({
  spec,
  value,
  busy,
  handlers,
}: {
  spec: EnvVarSpec;
  value: EnvValue;
  busy: boolean;
  handlers: ControlHandlers;
}) {
  const { onSet, onClear, onRequestSecret } = handlers;
  const isSet = value.state !== "absent";

  if (spec.safety.blocked_reason) {
    return <span className="envvar-readonly">read-only</span>;
  }

  if (value.state === "custom" || value.state === "custom_opaque") {
    return (
      <CustomValueControl
        spec={spec}
        value={value}
        busy={busy}
        onClear={onClear}
      />
    );
  }

  // Branch on the VALUE first, not only on the spec's `secret` flag. Core
  // also emits `secret_set` for a credential-shaped value sitting under a
  // variable nobody classified as credential-bearing — keying on the flag
  // alone dropped those through to the ordinary chooser, which then showed
  // an empty editable field for a variable that is very much set.
  if (spec.safety.secret || value.state === "secret_set") {
    return (
      <span className="envvar-scalar">
        <SecretChooser
          spec={spec}
          isSet={isSet}
          disabled={busy}
          onRequestStore={(handle) => onRequestSecret(spec.name, handle)}
        />
        {isSet ? (
          <ClearButton name={spec.name} busy={busy} onClear={onClear} />
        ) : null}
      </span>
    );
  }

  // Tri-state when the variable documents an off VALUE; two-state when off
  // just means removing the key. Keyed on that rather than on `off === "0"`,
  // which classified every `true`/`false` variable as two-state.
  if (spec.control === "toggle" && spec.off && spec.off !== UNSET) {
    const tri: TriState = value.state === "known" ? value.value : UNSET;
    return (
      <ToggleTriState
        value={tri}
        onLiteral={spec.on ?? "1"}
        offLiteral={spec.off}
        disabled={busy}
        label={`${spec.name} state`}
        onChange={(next) =>
          next === UNSET ? onClear(spec.name) : onSet(spec.name, next)
        }
      />
    );
  }

  if (spec.control === "toggle") {
    return (
      <ToggleTwoState
        checked={isSet}
        disabled={busy}
        label={spec.name}
        // The documented on-literal, not a hardcoded "1".
        onChange={(on) =>
          on ? onSet(spec.name, spec.on ?? "1") : onClear(spec.name)
        }
      />
    );
  }

  if (spec.control === "enum") {
    return (
      <EnumChooser
        spec={spec}
        value={value.state === "known" ? value.value : null}
        disabled={busy}
        onSelect={(v) => onSet(spec.name, v)}
        onClear={() => onClear(spec.name)}
      />
    );
  }

  return (
    <span className="envvar-scalar">
      <ScalarChooser
        spec={spec}
        value={value.state === "known" ? value.value : ""}
        disabled={busy}
        commitMode={commitModeFor(spec)}
        onCommit={(v) => onSet(spec.name, v)}
      />
      {isSet ? (
        <ClearButton name={spec.name} busy={busy} onClear={onClear} />
      ) : null}
    </span>
  );
}
