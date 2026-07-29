import {
  type ComponentPropsWithoutRef,
  type CSSProperties,
  type ReactNode,
  type Ref,
  useId,
  useState,
} from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";

/**
 * Everything a native `<input>` takes, minus the two the wrapper owns.
 *
 * The prop list used to be hand-written, which is why it accepted
 * `aria-label` and `list` but not `id`, `name`, `required`, `autoComplete`,
 * `inputMode`, `min`, `max`, `step`, or `aria-invalid` — each one a caller
 * either did without or worked around. Deriving from the native props means
 * the next attribute somebody needs already works.
 */
type NativeInputProps = Omit<
  ComponentPropsWithoutRef<"input">,
  // The wrapper draws the chrome, so its style is not the input's.
  "style" | "className" | "ref"
>;

interface InputProps extends NativeInputProps {
  /** Leading icon, inside the field chrome. */
  glyph?: NfIcon;
  /**
   * Trailing slot. **Non-interactive content only** — a unit label, a count,
   * a hint. Put buttons beside the `<Input>`, not inside it: the trailing
   * slot sits within the field's own click target, so a control here
   * competes with the input for clicks and focus.
   */
  suffix?: ReactNode;
  /** Styles for the wrapper (width, flex), not the inner `<input>`. */
  style?: CSSProperties;
  /**
   * Forward a ref to the inner `<input>` element. Callers programmatic-
   * ally focus the field (e.g. ⌘F in the Config section) by assigning
   * the ref and calling `.focus()` / `.select()`.
   */
  inputRef?: Ref<HTMLInputElement>;
}

/**
 * Minimal mono input. Leading `glyph` and trailing `suffix` slots.
 * Accent-bordered on focus. Matches the `--input-height` (32px)
 * token so inputs and buttons align in toolbars.
 *
 * # Why the wrapper is not a `<label>`
 *
 * It used to be, which broke two things at once. A caller that supplies its
 * own `<label htmlFor=…>` ended up with two labels for one control, and the
 * component had no way to forward an `id` for that `htmlFor` to point at.
 * And a `<label>` claims every click inside it for its control, so anything
 * interactive in `suffix` — there is an `IconButton` in one today — got
 * ambiguous click and focus behaviour. A `<div>` with `:focus-within`
 * chrome has neither problem.
 *
 * The input carries `pm-focus` so the design system's keyboard-focus ring
 * applies: the inline-style approach these primitives use cannot express
 * `:focus-visible` itself, and the accent border alone fires on mouse focus
 * too, so it is chrome rather than a focus indicator.
 */
export function Input({
  glyph,
  suffix,
  style,
  inputRef,
  type = "text",
  onFocus,
  onBlur,
  id,
  ...rest
}: InputProps) {
  const [focused, setFocused] = useState(false);
  const generatedId = useId();
  const inputId = id ?? generatedId;
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        height: "var(--input-height)",
        padding: "0 var(--sp-10)",
        background: "var(--bg-raised)",
        border: `var(--bw-hair) solid ${focused ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-2)",
        transition: "border-color var(--dur-fast) var(--ease-linear)",
        opacity: rest.disabled ? "var(--opacity-dimmed)" : 1,
        ...style,
      }}
    >
      {glyph && (
        // Clicking the icon focuses the field, which is what the old
        // `<label>` wrapper gave for free. `onMouseDown` rather than
        // `onClick` so focus lands before the browser moves it elsewhere.
        <span
          aria-hidden="true"
          style={{ display: "inline-flex", cursor: "text" }}
          onMouseDown={(e) => {
            e.preventDefault();
            (
              e.currentTarget.parentElement?.querySelector("input") ?? null
            )?.focus();
          }}
        >
          <Glyph g={glyph} color="var(--fg-faint)" />
        </span>
      )}
      <input
        {...rest}
        id={inputId}
        ref={inputRef}
        type={type}
        className="pm-focus"
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        style={{
          flex: 1,
          minWidth: 0,
          border: "none",
          outline: "none",
          background: "transparent",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      />
      {suffix}
    </div>
  );
}
