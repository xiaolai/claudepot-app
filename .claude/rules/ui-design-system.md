---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# UI Design System — Tokens and Defaults

This is the base layer: tokens, current numeric defaults, and the index
to the rest of the system. Everything here is either platform-inherited
(and therefore stable) or Claudepot house style (and therefore
versioned — re-baseline when macOS ships a major release).

## Register: macOS dialect, not strict HIG

Claudepot is a Tauri-based dev utility, not an AppKit app. We stopped
pretending to be 100% native — that pretense is where the uncanny-valley
output came from. The register is *quiet dev dialect*: platform color
tokens for light/dark/accent, but typography and chrome are our own
— JetBrainsMono Nerd Font Mono everywhere, with a compact, deliberate
size rhythm. Precedents: Warp terminal, Zed, Lapce, 1Password 8.

## Load order

| Layer | File | Authority |
|---|---|---|
| Why | `design-principles.md` | Non-negotiable. Wins conflicts. |
| What to imitate | `design-references.md` | Scoped to typography, chrome, proportions. |
| Which surface | `feedback-ladder.md` | Deterministic state-to-surface mapping. |
| How to compose | `design-patterns.md` | Recipes you can copy. |
| Which tokens | this file | Current defaults. |
| A11y floor | `accessibility.md` | Not the ceiling. |
| Code conventions | `react-components.md` | Component file shape. |

Read top to bottom for new work. For a small fix, start at the layer
that matches the problem.

## Tokens — platform-inherited

These are backed by `-apple-system-*` CSS tokens. They auto-adapt to
light/dark, the user's accent, and accessibility settings. Do not
hardcode values for any of these.

| Token | Source | Purpose |
|---|---|---|
| `--bg` | `-apple-system-background` | Content background |
| `--surface` | `-apple-system-secondary-background` | Modal / card |
| `--border` | `-apple-system-separator` | 0.5 px borders |
| `--text` | `-apple-system-label` | Primary text |
| `--muted` | `-apple-system-secondary-label` | Metadata, labels |
| `--tertiary` | `-apple-system-tertiary-label` | Disabled, placeholders |
| `--accent` | `AccentColor` | Active state, primary action |
| `--accent-text` | `AccentColorText` | Text on accent fill |
| `--ok` | `-apple-system-green` | Success only |
| `--bad` | `-apple-system-red` | Error only |
| `--warn` | `-apple-system-orange` | Warning only |

## Tokens — Claudepot-derived

These are our house style, derived from the platform tokens. They
change if the platform or our taste changes.

| Token | Derivation | Purpose |
|---|---|---|
| `--chrome` | `color-mix(--bg 90 %, --muted)` | Rail + sidebar (subtly distinct from content) |
| `--hover-bg` | accent at 6 % | List row hover |
| `--accent-weak` | accent at 12–18 % | Active-state background |
| `--ok-weak` | green at 12 % | Success badge background |
| `--bad-weak` | red at 12 % | Error badge background |
| `--warn-weak` | orange at 12 % | Warning badge background |
| `--focus-ring` | accent at 30 %, 3 px | Focus ring |

## Mental model: the 12-step scale (Radix)

When you reach for a new shade, think in terms of the 12-step semantic
scale Radix Colors pioneered and the whole modern web has adopted.
Each step has a specific role; a color without a role is noise.

| Step | Role | Claudepot example |
|---|---|---|
| 1 | **App background** | `--bg` |
| 2 | **Subtle background** | `--chrome` (rail/sidebar) |
| 3 | **UI element bg — rest** | button default bg |
| 4 | **UI element bg — hover** | `--hover-bg` |
| 5 | **UI element bg — active / selected** | `--accent-weak` (selected row) |
| 6 | **Subtle border, non-interactive** | `--border` (0.5 px separators) |
| 7 | **UI element border, focus rings** | (accent, used for focus) |
| 8 | **Hovered UI element border** | (not yet used; future) |
| 9 | **Solid background (high chroma)** | `--accent` (primary button fill) |
| 10 | **Hovered solid** | `--accent` × 1.05 brightness |
| 11 | **Low-contrast text** | `--muted` |
| 12 | **High-contrast text** | `--text` |

Rule: before adding a new color, identify which step it occupies. If
it doesn't fit a step, it's probably the wrong color. Two colors
at the same step for the same semantic role is duplication — merge.

Reference: *Radix Colors — Understanding the Scale*
([radix-ui.com/colors/docs/palette-composition/understanding-the-scale](https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale)).

## Semantic-pair pattern (shadcn/ui)

Every background token has a paired foreground for text/icons that
sit on it. Name them as a pair so you can't apply a background
without thinking about readable content on top.

| Background | Foreground | Usage |
|---|---|---|
| `--bg` | `--text` | Content surface |
| `--chrome` | `--text` | Rail + sidebar |
| `--surface` | `--text` | Modals, cards |
| `--accent` | `--accent-text` | Primary button fill |
| `--bad` | `white` | Destructive button fill |

Rule: if you introduce a new surface token `--X`, also define (or
confirm) the foreground. Inaccessible contrast (< WCAG AA 4.5:1) is
a BLOCK finding in review.

Reference: *shadcn/ui — Theming*
([ui.shadcn.com/docs/theming](https://ui.shadcn.com/docs/theming)).

## State suffix convention (Primer)

Interactive component tokens use explicit state suffixes. Name the
state; don't encode it only in selector context.

Allowed suffixes: `-rest`, `-hover`, `-active`, `-disabled`.

Example:

```css
--btn-default-bg-rest:     var(--surface);
--btn-default-bg-hover:    var(--hover-bg);
--btn-default-bg-active:   var(--accent-weak);
--btn-default-bg-disabled: color-mix(in srgb, var(--surface) 60%, transparent);
```

For Claudepot we don't yet ship per-component tokens (our class-
scoped CSS handles it), but when the app grows to where it matters
— theme switching, multiple button variants — follow this pattern.

Reference: *Primer Foundations — Color*
([primer.style/foundations/primitives/color](https://primer.style/foundations/primitives/color)).

## Motion tokens

Duration scale — three values, one role each:

| Token | Value | Use |
|---|---|---|
| `--dur-fast` | 80 ms | Hover/focus state changes |
| `--dur-base` | 120 ms | Standard transitions (backgrounds, color) |
| `--dur-slow` | 240 ms | Modals, large surfaces entering/leaving |

Easing — three values:

| Token | Value | Use |
|---|---|---|
| `--ease-out` | `cubic-bezier(0.2, 0.8, 0.2, 1)` | Default; feels responsive |
| `--ease-in-out` | `cubic-bezier(0.4, 0, 0.2, 1)` | Symmetric (loading spinners) |
| `--ease-in` | `cubic-bezier(0.4, 0, 1, 1)` | Exit/dismiss |

Rule: animate only `opacity` and `transform`. Wrap all new animations
in `@media (prefers-reduced-motion: no-preference)`.

## Elevation (shadow) tokens

Four levels — flat by default; elevate only where it communicates
hierarchy.

| Token | Value | Use |
|---|---|---|
| `--shadow-0` | none | Default (list rows, chrome) |
| `--shadow-1` | `0 0.5px 1px rgba(0,0,0,.06)` | Banners, inline callouts |
| `--shadow-2` | `0 4px 16px rgba(0,0,0,.15)` | Popovers, dropdowns, context menus |
| `--shadow-3` | `0 20px 60px rgba(0,0,0,.20)` | Modals |

Never use shadows on list rows, detail grid rows, or anything that
should read as flat. List hover is color fill, never shadow.

## Rule: no raw colors

Every color in component CSS comes from the tables above. No raw hex,
rgb, hsl, or named colors. This is a BLOCK-level rule in
`design-review.md`.

If a needed color isn't here, add it to the table with justification
before using it.

## Typography

**One family, everywhere.** JetBrainsMono Nerd Font Mono. The `Mono`
suffix means every glyph — including the ~9,000 Nerd Font icons —
is forced to the monospace cell, so lists and columns align
predictably.

```
--font:      "JetBrainsMono NF", ui-monospace, "SF Mono",
             SFMono-Regular, Menlo, monospace;
--font-mono: (same — body IS mono)
```

Font files live in `public/fonts/JetBrainsMonoNerdFontMono-*.woff2`
at weights 400, 500, 600, 700. No proportional fallback — the mono
body is the aesthetic signal. This is the dev-dialect register (Warp,
Zed, Lapce), not a retreat from taste.

Size scale — current house style. Invented sizes are a BLOCK finding
in review.

| Role | Size | Weight |
|---|---|---|
| Page title | 14 px | 600 |
| Section heading | 12 px | 600 |
| Subheading / sidebar label | 10 px | 600 uppercase, 0.04em tracking |
| Body / button | 11 px | 400 (500 for primary) |
| List row primary | 11 px | 500 (600 when selected) |
| Metadata / hint | 10 px | 400 |
| Badge / tag | 9 px | 600 |

Scale: `9 · 10 · 11 · 12 · 14`. Five sizes, no more.

**Line-height — exactly three values:**
- `1.0` — icons, badges, single-line chrome
- `1.3` — body, list rows
- `1.5` — paragraphs, empty-state copy, descriptions

**Letter-spacing — exactly three values:**
- `-0.01em` — titles, section headings (slight tightening)
- `0` — body (default)
- `0.04em` — uppercase labels

**Baseline applied to `<body>`:**
```css
-webkit-font-smoothing: antialiased;
-moz-osx-font-smoothing: grayscale;
text-rendering: optimizeLegibility;
font-feature-settings: "kern", "liga", "calt";
font-variant-numeric: tabular-nums;
```
Global `tabular-nums` means every number in the UI lines up in columns
by default — no per-class opt-in needed.

## Spacing

4 px grid: `2 · 4 · 6 · 8 · 10 · 12 · 16 · 20 · 24 · 32`.

House style. Violations are WARN, not BLOCK, in review.

## Border radius

| Element | Radius |
|---|---|
| Buttons, inputs, list rows | 6 px |
| Banners | 8 px |
| Toasts | 10 px |
| Modals | 12 px |
| Pills, badges | 999 px |

House style.

## Transitions

Use the `--dur-*` and `--ease-*` tokens. Default is
`var(--dur-base) var(--ease-out)` — i.e., 120 ms, feels responsive.
Only animate `opacity` and `transform`; never layout properties.

Wrap all animations in `@media (prefers-reduced-motion: no-preference)`.

## Button variants

Exactly five canonical variants — match shadcn/ui / Radix / Primer
naming so the taxonomy is familiar to anyone who's touched a modern
web design system.

| Variant | Visual | When |
|---|---|---|
| `.btn.primary` | Accent fill, white text | The one primary action of the view |
| `.btn` (default) | Surface fill, 0.5 px border | Secondary / neutral action |
| `.btn.outline` | Transparent fill, 0.5 px border, border-colored text | Tertiary / alternate action |
| `.btn.ghost` | Transparent at rest, hover fill only | Toolbar icon buttons, overflow menus |
| `.btn.danger` | Bad fill or bad-colored border | Destructive action (delete, clean, reset) |

Height scale: `sm` 24 px, `md` 28 px (default), `lg` 32 px. No `xl`.

Rule: exactly **one** `.btn.primary` visible per view. Two primaries
means you haven't decided which action matters.

Reference: *shadcn/ui — Button*
([ui.shadcn.com/docs/components/button](https://ui.shadcn.com/docs/components/button)).

## Icons

Nerd Font glyphs via the `<Icon>` component (`src/components/Icon.tsx`),
drawn from the 39-entry map in `src/icons.ts`. Size via the `size`
prop, color inherits.

| Context | Size |
|---|---|
| Nav rail | 18 px |
| Toolbar / section heading | 14 px |
| Inline / buttons | 14 px |
| List row accent | 12–13 px |
| Empty state | 28–32 px |

No emoji in place of icons. No mixing icon libraries. No colored
icons outside the semantic set (`ok` / `bad` / `warn`).

## Cursor and selection

- `cursor: default` globally. Pointer cursor is for hyperlinks only.
- `user-select: none` on UI chrome. Only content text (paths, tokens,
  `.selectable`) is selectable.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Cmd+R | Refresh |
| Cmd+N | Add (account) |
| Cmd+, | Settings |
| Cmd+1/2/3… | Switch section |
| Cmd+F | Focus search |
| Escape | Close modal |

Do not fire while a modal is open or an input has focus. Standard
Cmd+Q/W/H/M come free from Tauri.

## macOS Tahoe / Liquid Glass — future work

These numeric defaults (12 px body, 6 px radius, 0.5 px borders) are
current house style, versioned against today's macOS. Re-baseline when
macOS ships a major release (Tahoe etc.). Do not refactor preemptively.

Claudepot uses a native OS-drawn title bar (not overlay/unified), so
Liquid Glass treatment of the title bar is automatic once the OS
supports it — no Tauri config change needed on our side.

## Shortlist of anti-patterns

Design-level failures — the full list with replacement recipes lives
in `design-patterns.md`. BLOCK-level unless noted.

- Two navigation surfaces competing.
- Icon-only segmented controls without visible labels.
- Zero-value metadata (`0 sessions · …`).
- Internal identifiers on the primary detail grid.
- Disabled buttons without inline reason.
- Raw hex / rgb colors in component CSS.
- `cursor: pointer` on buttons; `::-webkit-scrollbar` overrides.
- Box shadows on list items; radius > 8 px on non-modals (WARN).
- Invented font sizes outside the scale above (WARN).
