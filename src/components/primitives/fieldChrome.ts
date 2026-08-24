import type { CSSProperties } from "react";

/**
 * The border, background and focus treatment every text field wears.
 *
 * Extracted because there are now two controls that need it and the
 * global reset gives neither of them anything: `tokens.css` sets only
 * `font` and `color` on `input, textarea`, so a bare field renders with
 * the user-agent border — a 1px grey rule on a textarea and a 2px inset
 * bevel on an input in WebKit. That is what `QuickPromptsPane`'s two
 * fields were doing, in the same pane as fields that went through
 * `Input` and therefore had the design system's hairline.
 *
 * **Not a global `input, textarea` rule**, which is the obvious fix and
 * the wrong one: `Input` draws its chrome on a WRAPPER and clears the
 * inner element's border inline, so a global border would put a second
 * one inside the first.
 *
 * Two functions rather than one object because the chrome depends on
 * focus and disabled state, and the shell and the control want opposite
 * things — the shell paints, the control gets out of the way.
 */

/** The wrapper: what the user sees as "the field". */
export function fieldShell(opts: {
  focused: boolean;
  disabled?: boolean;
  /** `false` for a multiline field, whose height is its content. */
  fixedHeight?: boolean;
}): CSSProperties {
  const { focused, disabled, fixedHeight = true } = opts;
  return {
    display: "flex",
    alignItems: fixedHeight ? "center" : "flex-start",
    gap: "var(--sp-8)",
    ...(fixedHeight ? { height: "var(--input-height)" } : {}),
    padding: fixedHeight ? "0 var(--sp-10)" : "var(--sp-6) var(--sp-10)",
    background: "var(--bg-raised)",
    border: `var(--bw-hair) solid ${focused ? "var(--accent-border)" : "var(--line)"}`,
    borderRadius: "var(--r-2)",
    transition: "border-color var(--dur-fast) var(--ease-linear)",
    opacity: disabled ? "var(--opacity-dimmed)" : 1,
  };
}

/**
 * The control inside it.
 *
 * `border`/`outline`/`background` are cleared here rather than relied on
 * from a reset, because there is no reset — see the module note.
 */
export function fieldControl(): CSSProperties {
  return {
    flex: 1,
    minWidth: 0,
    border: "none",
    outline: "none",
    background: "transparent",
    fontSize: "var(--fs-sm)",
    color: "var(--fg)",
  };
}
