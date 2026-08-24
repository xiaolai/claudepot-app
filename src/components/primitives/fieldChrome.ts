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
 *
 * ## The focus indicator is an outline, not the button ring
 *
 * `tokens.css` documents two DIFFERENT focus treatments and says which
 * is for which: *"Focus-ring offsets — outline pattern (input/list/row
 * controls). The box-shadow ring (filled chrome controls) keeps using
 * `--focus-ring` above."* `Input` used to ignore that split — its inner
 * element carried `pm-focus`, which pulls in the 3px `--focus-ring`
 * box-shadow meant for buttons — stacked on top of the wrapper's OWN
 * border turning accent-coloured. Two indicators at once, and the
 * box-shadow one had nowhere to go: the wrapper sets no vertical
 * padding, so the ring bled 2px past the pill's top and bottom edges
 * instead of being contained by it. It read as one heavy, doubled box
 * rather than a single crisp ring.
 *
 * The fix matches the pattern already established elsewhere in this
 * shard — `.settings-input:focus-visible` in `settings.css`, and the
 * same pair in `envvars.css` / `projects.css` / `banners.css`: an
 * `outline` at `--bw-focus` (2px, half the box-shadow ring's weight),
 * flush against the border with no offset. The border itself stops
 * changing colour on focus, so exactly one thing happens when a field
 * gains focus, matching every sibling control in the app rather than
 * inventing a second local treatment.
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
    border: "var(--bw-hair) solid var(--line)",
    borderRadius: "var(--r-2)",
    outline: focused ? "var(--bw-focus) solid var(--accent)" : "none",
    outlineOffset: "var(--focus-outline-offset-flat)",
    opacity: disabled ? "var(--opacity-dimmed)" : 1,
  };
}

/**
 * The control inside it.
 *
 * `border`/`outline`/`background` are cleared here rather than relied on
 * from a reset, because there is no reset — see the module note.
 * `outline: none` is what suppresses the browser's OWN default focus
 * ring on the native element — the wrapper draws the one the user sees.
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
