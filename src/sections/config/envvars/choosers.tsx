// The chooser appropriate to each control type.
//
// Two rules shape every one of them:
//
//  1. **Unset is not a value.** CC's default for nearly every variable
//     is the key being absent. So clearing is always its own action,
//     never "blank the field" — an explicit empty string is a third,
//     distinct process state, and a text box showing "" cannot be told
//     apart from one showing nothing.
//
//  2. **Commit escalates with the stakes.** Blur is an *accidental*
//     gesture. Tabbing out of a field must not store a token or
//     redirect an API endpoint, so the hazardous and secret rows
//     require an explicit action and a confirmation. Safe text and
//     numbers commit on Enter or blur, where the cost of a slip is a
//     value you can see and change back.

import { useEffect, useId, useRef, useState } from "react";
import { Button, Input } from "../../../components/primitives";
import type { EnvVarSpec } from "../../../types/ccEnv";

/** Which commit gesture a row gets. Derived, never hand-set per row. */
export type CommitMode = "immediate" | "apply" | "store-secret";

export function commitModeFor(spec: EnvVarSpec): CommitMode {
  if (spec.safety.secret) return "store-secret";
  if (spec.safety.hazards.some((h) => h !== "unknown")) return "apply";
  return "immediate";
}

/* ── toggles ──────────────────────────────────────────────────────── */

/**
 * Two-state switch: off means *removing the key*, because that is what
 * off means for a variable whose only documented value is `1`.
 */
export function ToggleTwoState({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className="envvar-switch"
      data-on={checked ? "true" : "false"}
      onClick={() => onChange(!checked)}
    >
      <span className="envvar-switch-knob" aria-hidden="true" />
    </button>
  );
}

/** `"unset"`, or one of the variable's own documented literals. */
export type TriState = string;

/** The sentinel for "no key at all", distinct from any documented value. */
export const UNSET: TriState = "unset";

/**
 * Three-segment control for a variable whose doc defines all three of
 * unset, off and on as distinct behaviours.
 *
 * The two segment labels are the variable's **documented literals** — `0`/`1`
 * for most, `false`/`true` for the eleven whose prose uses that vocabulary.
 * Both work at runtime, but a control that shows `1` while the documentation
 * says `true` is asking the user to trust a translation nobody wrote down.
 *
 * A `radiogroup` with arrow-key traversal, not three buttons: three
 * buttons announce as three unrelated controls and give no sense of
 * "one of these is chosen".
 */
export function ToggleTriState({
  value,
  onLiteral,
  offLiteral,
  disabled,
  label,
  onChange,
}: {
  value: TriState;
  onLiteral: string;
  offLiteral: string;
  disabled?: boolean;
  label: string;
  onChange: (next: TriState) => void;
}) {
  const options: { key: TriState; text: string; hint: string }[] = [
    { key: UNSET, text: "Unset", hint: "Remove the key — Claude Code decides" },
    { key: offLiteral, text: offLiteral, hint: "Explicitly off" },
    { key: onLiteral, text: onLiteral, hint: "Explicitly on" },
  ];
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  const move = (from: number, delta: number) => {
    const next = (from + delta + options.length) % options.length;
    onChange(options[next].key);
    refs.current[next]?.focus();
  };

  return (
    <div role="radiogroup" aria-label={label} className="envvar-tristate">
      {options.map((o, i) => (
        <button
          key={o.key}
          type="button"
          role="radio"
          aria-checked={value === o.key}
          title={o.hint}
          disabled={disabled}
          // Roving tabindex: the group is one tab stop, arrows move
          // within it.
          tabIndex={value === o.key ? 0 : -1}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className="envvar-tristate-seg"
          onClick={() => onChange(o.key)}
          onKeyDown={(e) => {
            if (e.key === "ArrowRight" || e.key === "ArrowDown") {
              e.preventDefault();
              move(i, 1);
            } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
              e.preventDefault();
              move(i, -1);
            }
          }}
        >
          {o.text}
        </button>
      ))}
    </div>
  );
}

/* ── enum ─────────────────────────────────────────────────────────── */

export function EnumChooser({
  spec,
  value,
  disabled,
  onSelect,
  onClear,
}: {
  spec: EnvVarSpec;
  value: string | null;
  disabled?: boolean;
  onSelect: (v: string) => void;
  onClear: () => void;
}) {
  // A sentinel the generator can never emit as a real value: its
  // literal shape is /[a-z0-9][a-z0-9._-]{0,15}/, so nothing starting
  // with a parenthesis can collide with one.
  const UNSET = "(unset)";
  return (
    <select
      className="envvar-select"
      aria-label={`${spec.name} value`}
      disabled={disabled}
      value={value ?? UNSET}
      onChange={(e) => {
        if (e.target.value === UNSET) onClear();
        else onSelect(e.target.value);
      }}
    >
      <option value={UNSET}>(unset — CC default)</option>
      {(spec.values ?? []).map((v) => (
        <option key={v} value={v}>
          {v}
        </option>
      ))}
    </select>
  );
}

/* ── number / text / secret ───────────────────────────────────────── */

/**
 * Number and text share a body; only the keyboard hint and the
 * placeholder differ. The placeholder carries the documented default —
 * shown, never written. Writing it would pin today's number into
 * settings and override whatever CC changes it to later.
 */
export function ScalarChooser({
  spec,
  value,
  disabled,
  commitMode,
  onCommit,
}: {
  spec: EnvVarSpec;
  value: string;
  disabled?: boolean;
  commitMode: CommitMode;
  onCommit: (next: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const dirty = draft !== value;
  const immediate = commitMode === "immediate";
  // Enter and blur both fire when a user presses Enter and the field then
  // loses focus. Without this the same value is submitted twice — harmless
  // for the file, but it doubles the write and the "Saved" announcement.
  //
  // Scoped to that one sequence: the guard is armed by a commit and cleared
  // by the next keystroke, so cancelling a confirmation and pressing Apply
  // again still works. A guard that outlived the gesture would strand the
  // user with a draft they could not resubmit without editing it first.
  const submitted = useRef<string | null>(null);

  const commit = (next: string) => {
    // Nothing to write. Without this, Enter on an untouched absent field
    // writes an explicit `""` — a real, distinct process state the user
    // never asked for.
    if (next === value) return;
    if (submitted.current === next) return;
    submitted.current = next;
    onCommit(next);
  };

  return (
    <span className="envvar-scalar">
      <Input
        value={draft}
        disabled={disabled}
        type="text"
        aria-label={`${spec.name} value`}
        placeholder={spec.default || spec.format || "unset"}
        style={{ minWidth: "var(--input-width-md)" }}
        onChange={(e) => {
          submitted.current = null;
          setDraft(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && immediate) commit(draft);
          if (e.key === "Escape") setDraft(value);
        }}
        onBlur={() => {
          // Only where a slip is cheap. Hazardous rows require Apply.
          if (immediate && dirty) commit(draft);
        }}
      />
      {spec.unit ? <span className="envvar-unit">{spec.unit}</span> : null}
      {!immediate && dirty ? (
        // `subtle`, not `solid`: rules/design.md allows one primary action
        // per view, and a 308-row list would otherwise offer hundreds.
        //
        // Apply always re-submits: the previous attempt may have been
        // cancelled at the confirmation or rejected by the backend, and a
        // button that silently does nothing is worse than one that retries.
        <Button
          variant="subtle"
          onClick={() => {
            submitted.current = null;
            commit(draft);
          }}
        >
          Apply
        </Button>
      ) : null}
    </span>
  );
}

/** How the pane reads and then destroys a pasted secret. */
export interface SecretHandle {
  /** Read the pasted value, or `null` if the field is gone — the row was
   *  filtered away or unmounted while the confirmation was open. `null` is
   *  not the same as `""`: the backend accepts an empty string as a real
   *  value, so a lost field must abort rather than silently overwrite a
   *  stored credential with nothing. */
  read: () => string | null;
  /** Blank the field. Called on every outcome, including cancel. */
  wipe: () => void;
}

/**
 * Write-only secret field.
 *
 * `rules/architecture.md` permits secrets *inbound* and forbids them
 * *outbound*, so a field that accepts a paste and reports only
 * `set` / `not set` is compliant. The real objection was never IPC, it
 * was storage: CC keeps these values in plaintext. That is answered by
 * making the write deliberate rather than by refusing it — refusing
 * outright just moves the same write into a text editor, where the user
 * gets no warning and no zeroization.
 *
 * # Why the plaintext never enters React state
 *
 * The write is gated behind a confirmation. A controlled field would put the
 * plaintext in this component's state, and passing it up would copy it into
 * the *parent's* pending-confirmation state — where it would sit until the
 * user answered, and stay if they never did. `rules/architecture.md` asks
 * that renderer state not outlive the single bridge call.
 *
 * So the field is **uncontrolled**: the value lives only in the DOM node,
 * the parent receives a [`SecretHandle`] it can read once at the moment of
 * the write, and `wipe()` runs on every outcome — including cancel, which
 * the earlier shape did not cover at all. The only React state here is
 * whether the field is empty, which is what disables the button.
 *
 * `Input` renders uncontrolled when no `value` is passed; it used to force
 * `value ?? ""`, which is why an earlier attempt at this silently reset the
 * field on every keystroke.
 */
export function SecretChooser({
  spec,
  isSet,
  disabled,
  onRequestStore,
}: {
  spec: EnvVarSpec;
  isSet: boolean;
  disabled?: boolean;
  onRequestStore: (handle: SecretHandle) => void;
}) {
  const field = useRef<HTMLInputElement | null>(null);
  const [empty, setEmpty] = useState(true);
  const id = useId();
  return (
    <span className="envvar-scalar">
      <span className="envvar-badge" data-tone={isSet ? "set" : "unset"} id={id}>
        {isSet ? "set" : "not set"}
      </span>
      <Input
        type="password"
        inputRef={field}
        disabled={disabled}
        aria-label={`${spec.name} new value`}
        aria-describedby={id}
        placeholder="paste to replace"
        style={{ minWidth: "var(--input-width-md)" }}
        onChange={(e) => setEmpty(e.target.value.length === 0)}
      />
      <Button
        variant="subtle"
        disabled={disabled || empty}
        onClick={() =>
          onRequestStore({
            read: () => field.current?.value ?? null,
            wipe: () => {
              if (field.current) field.current.value = "";
              setEmpty(true);
            },
          })
        }
      >
        Store in settings.json
      </Button>
    </span>
  );
}
