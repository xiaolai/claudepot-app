---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# No Raw Values in UI Code

Every color, size, font, shadow, radius, opacity, duration, easing,
border width, and z-index that appears in a paper-mono UI component
must come from a token defined in `src/styles/tokens.css`. Raw
literals are a BLOCK-level review finding and a runtime regression
risk — both themes and the `prefers-contrast: more` / reduced-motion
switches rely on the token layer.

This rule is hard. "But it's just one number" is not an exception.

## What counts as a raw value (forbidden)

A raw value is any literal that renders as a CSS physical property
without going through a named token.

### Colors

**Forbidden:** `"#fff"`, `"#000"`, `"#abc123"`, `"rgb(…)"`,
`"rgba(…)"`, `"hsl(…)"`, `"hsla(…)"`, `"oklch(…)"` literal in
component code, and CSS named colors (`"white"`, `"black"`, `"red"`,
`"transparent"` is the **only allowed** keyword).

**Allowed:**

```tsx
color: "var(--fg)"
background: "var(--bg-raised)"
border: "var(--bw-hair) solid var(--line)"
color: "transparent"          // keyword — allowed
```

### Font sizes

**Forbidden:** any numeric literal or px string in a `fontSize`
property (`fontSize: 10`, `fontSize: "14px"`).

**Allowed:**

```tsx
fontSize: "var(--fs-xs)"      // 11px — token
fontSize: "var(--fs-2xs)"     // 10px — token
```

Scale: `--fs-4xs 8 · 3xs 9 · 2xs 10 · xs 11 · sm 12 · base 13 · md
14 · md-lg 15 · lg 18 · xl 22 · 2xl 28`. Values outside the scale
must be added to `tokens.css` first.

### Spacing (padding, margin, gap, row/column-gap)

**Forbidden:** `padding: 14`, `padding: "10px 8px"`, `gap: 6`,
`marginTop: 4`.

**Allowed:**

```tsx
padding: "var(--sp-14)"
padding: "var(--sp-10) var(--sp-8)"
gap: "var(--sp-6)"
marginTop: "var(--sp-4)"
```

Scale: every integer px from 1 through 80 used anywhere in the UI is
exposed as `--sp-N` (e.g., `--sp-1: 1px`, `--sp-10: 10px`,
`--sp-64: 64px`). If you need a value that isn't defined, add it to
`tokens.css` with both light and dark considerations in mind.

### Component dimensions (width, height, min/max)

**Forbidden:** `width: 240`, `height: 38`, `maxWidth: 520`.

**Allowed:**

```tsx
width: "var(--sidebar-width)"        // 240px
height: "var(--chrome-height)"       // 38px
maxWidth: "var(--modal-width-lg)"    // 520px
width: "var(--avatar-xl)"            // 36px
height: "var(--btn-h-md)"            // 30px
```

Semantic tokens exist for: chrome/statusbar heights, sidebar/settings
widths, modal widths, content caps, avatar sizes, button/icon-button
heights, row heights, input height, Kbd dimensions, toggle
dimensions, filter-input width, banner min-width.

### Radii

**Forbidden:** `borderRadius: 6`, `borderRadius: "10px"`.

**Allowed:**

```tsx
borderRadius: "var(--r-2)"         // 6px — rows/buttons/inputs
borderRadius: "var(--r-3)"         // 10px — modals/cards
borderRadius: "var(--r-pill)"      // 999px — pills/toggles
borderRadius: "50%"                // explicit circle — allowed keyword
```

### Shadows

**Forbidden:** any `"0 …px …px rgba(…)"` literal.

**Allowed:**

```tsx
boxShadow: "var(--shadow-modal)"
boxShadow: "var(--shadow-popover)"
boxShadow: "var(--shadow-thumb)"
boxShadow: "var(--shadow-md)"
```

### Border widths

**Forbidden:** `border: "1px solid ..."`, `borderLeft: "2px solid ..."`.

**Allowed:**

```tsx
border: "var(--bw-hair) solid var(--line)"           // 1px
borderLeft: "var(--bw-strong) solid var(--accent)"    // 2px
borderLeft: "var(--bw-accent) solid var(--accent)"    // 3px
```

### Opacity

**Forbidden:** `opacity: 0.45`, `opacity: 0.7`.

**Allowed:**

```tsx
opacity: "var(--opacity-disabled)"
opacity: "var(--opacity-dimmed)"
opacity: "var(--opacity-quiet)"
opacity: "var(--opacity-segbar)"
opacity: 1       // fully opaque — allowed keyword-like
opacity: 0       // fully transparent — allowed keyword-like
```

### Durations and easings

**Forbidden:** `transition: "background 80ms linear"`,
`transition: "left 120ms ease-out"`.

**Allowed:**

```tsx
transition:
  "background var(--dur-fast) var(--ease-linear), color var(--dur-fast) var(--ease-linear)"
transition: "left var(--dur-base) var(--ease-linear)"
```

### Z-index

**Forbidden:** `zIndex: 40`, `zIndex: 200`.

**Allowed:**

```tsx
zIndex: "var(--z-sticky)" as unknown as number
zIndex: "var(--z-popover)" as unknown as number
zIndex: "var(--z-toast)" as unknown as number
zIndex: "var(--z-modal)" as unknown as number
```

TypeScript's `CSSProperties.zIndex` is typed as `number`, so the
`as unknown as number` cast is the accepted idiom — it keeps the
token reference intact at runtime.

### Line heights

**Forbidden:** `lineHeight: 1`, `lineHeight: 1.5`.

**Allowed:**

```tsx
lineHeight: "var(--lh-flat)"     // 1 — glyphs, badges
lineHeight: "var(--lh-tight)"    // 1.25
lineHeight: "var(--lh-body)"     // 1.5 — body
lineHeight: "var(--lh-loose)"    // 1.7
```

### Letter spacing

**Forbidden:** `letterSpacing: "-0.01em"`, `letterSpacing: "0.14em"`.

**Allowed:**

```tsx
letterSpacing: "var(--ls-tight)"     // -0.01em
letterSpacing: "var(--ls-wide)"      // 0.08em
letterSpacing: "var(--ls-wider)"     // 0.14em
letterSpacing: 0                      // keyword-like
```

### Fonts

**Forbidden:** hardcoded font family strings like
`"'JetBrains Mono', monospace"`.

**Allowed:**

```tsx
fontFamily: "var(--font)"
fontFamily: "inherit"           // keyword — allowed
```

### Font weights

Numeric weights `400`, `500`, `600`, `700` are explicit numeric
identifiers, not physical pixel values. They are **allowed as
literals**:

```tsx
fontWeight: 500
fontWeight: 600
```

If an app-wide weight shift is ever needed, add a token; until then,
numerics are fine.

## What counts as allowed literals

Not every number is a "value" that needs tokenizing. The following
literals are explicitly OK and should not be flagged:

- **`0`** (with or without unit) — zero is zero, universally.
- **`1` as flex / gridRow / zIndex on a sticky element** — structural
  scalars, not physical values. Prefer `var(--z-sticky)` for
  clarity.
- **Percentages** — `width: "100%"`, `left: "50%"`, `maxWidth: "100%"`.
- **`auto`**, **`inherit`**, **`unset`**, **`currentColor`**,
  **`transparent`** — CSS keywords.
- **`1fr`, `2fr`, `minmax(...)`, `repeat(...)`, `calc(...)`** — grid
  track sizing functions. Operands inside `calc`/`minmax` must
  themselves be tokens (`calc(100% + var(--sp-4))`).
- **Font weights** — see above.
- **Numeric props on a primitive** that the primitive re-renders as a
  token (e.g., `<Avatar size={36} />` maps internally to
  `var(--avatar-xl)` via a lookup table). The numeric is a prop
  *identifier*, not a raw CSS value — but semantic variants
  (`size="xl"`) are preferred.
- **Computed sub-pixel offsets from tokens** — e.g., Avatar's
  `fontSize: var(--avatar-initial-xl)`, which derives 18px from the
  `--avatar-xl` semantic.

## Where to allow / where to refuse

| Path | Policy |
|---|---|
| `src/styles/tokens.css` | **Only place** raw values live. Every token has both light and dark variants. **The only file allowed to open a `:root { }` block.** |
| `src/components/primitives/**` | All internal styles must cite tokens. Prop APIs may accept numbers as identifiers (e.g., `Avatar.size`) that resolve to tokens internally. |
| `src/shell/**` | Full token compliance — no escape hatch. |
| `src/sections/**` (paper-mono) | Full token compliance. |
| `src/components/*.tsx` (legacy, unported) | Full token compliance via the legacy-alias block in `tokens.css`. These files read `--surface`, `--border`, `--text`, etc. which are aliases — they must not introduce raw values. |
| `src/App.css` | Container for legacy **component classes only**. Must not declare tokens, open `:root`, or contain raw `rgba`/`#hex` literals outside of values that are themselves being migrated. The file is scheduled for piecewise deletion as its consumers migrate to inline tokenized styles. |

## Only `tokens.css` declares tokens

Declaring `:root { --x: ...; }` in any file other than `tokens.css`
creates a cascade collision: two competing declarations of the same
custom property, with the last-loaded one winning. That's what
broke the accent in the first rollout — `src/App.css` declared
`--accent: AccentColor` on `:root`, and since Vite happened to load
it after `tokens.css`, every component reading `var(--accent)` got
the OS system-accent (blue) instead of the paper-mono terracotta.

**The one-declaration rule:** tokens are declared exactly once, in
`src/styles/tokens.css`. Every other file reads. No `:root { ... }`
block anywhere else. `@media (prefers-color-scheme: dark) :root { }`
token overrides also live only in `tokens.css`.

This is BLOCK-level in `/design-review` Pass C. Grep:

```bash
# Must return zero:
rg -n ":root\s*\{" src/ --glob '!src/styles/tokens.css'

# App.css must not re-declare any of the canonical tokens.
# The trailing `:` requirement matches declaration syntax
# (`  --accent: value;`) and skips prose inside comments.
rg -n "^\s*--(accent|bg|surface|border|text|ok|bad|warn|focus-ring|shadow|font|dur-|ease-|selection|chrome|grouped-bg)[a-z0-9-]*\s*:" \
   src/App.css
```

## How to extend the token set

If you need a value not yet in `tokens.css`:

1. Stop. Confirm the new value is *semantic* (`--avatar-xl`,
   `--btn-h-lg`), not ad-hoc (`--misc-42`).
2. Add the token to `tokens.css` under the right section. Define
   both light and dark values when color-related.
3. If the value interacts with `prefers-contrast: more` or
   `prefers-reduced-transparency`, add the corresponding media
   block.
4. Update the appropriate rule doc
   (`ui-design-system.md` for scales, this file for forbidden
   categories) so future scaffolds pick it up.
5. Only after the token exists, reference it in components.

## How to audit

`rg` can catch most violations. Run all four before opening a PR.

```bash
# 1. No :root {} outside tokens.css — enforces one-declaration.
rg -n ":root\s*\{" src/ --glob '!src/styles/tokens.css'

# 2. App.css must not re-declare canonical tokens.
rg -n "^\s*--(accent|bg|surface|border|text|ok|bad|warn|focus-ring|shadow|font|dur-|ease-|selection|chrome|grouped-bg)\b" \
  src/App.css

# 3. Raw hex / rgb / hsl / oklch literals in component code.
#    App.css still hosts legacy rgba literals inside component rules
#    (being migrated); exempt it here and the next grep. Paper-mono
#    directories (primitives, shell, sections) must be clean.
rg -nE "(#[0-9a-fA-F]{3,8}\b|(rgba?|hsla?|oklch)\s*\()" \
  --glob '!src/styles/tokens.css' \
  --glob '!src/App.css' \
  --glob '!src/components/*.tsx' \
  src/

# 4. Numeric fontSize / padding / margin / gap / dimension in
#    paper-mono component code.
rg -nE "(fontSize|padding|margin|gap|width|height|borderRadius|lineHeight|zIndex|opacity):\s*['\"]?[1-9]" \
  --glob '!src/styles/tokens.css' \
  --glob '!src/App.css' \
  --glob '!src/components/*.tsx' \
  src/ \
  | rg -v "var\(--"
```

Greps 1 and 2 must return zero hits. Anything grep 3 or 4 surfaces
that isn't in the "allowed literals" list above is a BLOCK finding.

App.css's broader exemption (for rgba color literals inside
component classes) is temporary and shrinks each time a class
migrates. When the last paper-mono legacy alias is gone, delete
both App.css and the exemption line above.

## Review integration

- `/design-review` Pass C includes a BLOCK-level check against this
  rule for every touched file.
- `/component` scaffold emits only token-referenced styles.
- Tests assert on visible text and ARIA roles, never on style
  objects, so the token layer can move underneath without breaking
  coverage.
