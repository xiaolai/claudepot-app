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
| `--chrome` | `--surface` | Unified rail + sidebar background |
| `--hover-bg` | accent at 6 % | List row hover |
| `--accent-weak` | accent at 12–18 % | Active-state background |
| `--ok-weak` | green at 12 % | Success badge background |
| `--bad-weak` | red at 12 % | Error badge background |
| `--warn-weak` | orange at 12 % | Warning badge background |
| `--focus-ring` | accent at 30 %, 3 px | macOS focus ring |

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
| Section heading | 13 px | 600 |
| Subheading / sidebar label | 11 px | 600 uppercase, 0.04em tracking |
| Body / button | 12 px | 400 (500 for primary) |
| List row primary | 12 px | 500 (600 when selected) |
| Metadata / hint | 11 px | 400 |
| Badge / tag | 10 px | 600 |

Scale: `10 · 11 · 12 · 13 · 14`. Five sizes, no more.

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

`0.12s ease`. Only animate `opacity` and `transform`. Layout
properties (width, height, padding, margin) are never animated.

Wrap all animations in `@media (prefers-reduced-motion: no-preference)`.

## Icons

`lucide-react`, stroke-based. 16 px default via CSS (`svg.lucide`).

| Context | Stroke | Size |
|---|---|---|
| Toolbar / sidebar | 1.5 | 16 px |
| Inline / buttons | 1.5 | 14 px |
| Active indicator | 2.5 | same |
| Empty state | 1 | 28–32 px |

No emoji in place of icons. No mixing icon libraries. No colored icons
outside the semantic set (`ok` / `bad` / `warn`).

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

These numeric defaults (13 px body, 6 px radius, 0.5 px borders) are
current house style for macOS Sonoma/Sequoia, **not** timeless native
truth. When macOS Tahoe's Liquid Glass API lands in Tauri:

1. Remove `windowEffects: ["sidebar"]`; let the OS provide glass.
2. Re-baseline sizes against Tahoe control metrics (controls are
   slightly taller).
3. Audit this file; mark which values changed.

Do not refactor preemptively. Watch Tauri 2 release notes.

## Shortlist of anti-patterns

Design-level failures — the full list with replacement recipes lives
in `design-patterns.md`. BLOCK-level unless noted.

- Two navigation surfaces competing.
- Icon-only segmented controls without visible labels.
- Zero-value metadata (`0 sessions · …`).
- Internal identifiers on the primary detail grid.
- Disabled buttons without inline reason.
- Horizontal separator in the nav rail's top zone (breaks unified title bar).
- Raw hex / rgb colors in component CSS.
- `cursor: pointer` on buttons; `::-webkit-scrollbar` overrides.
- Box shadows on list items; radius > 8 px on non-modals (WARN).
- Invented font sizes outside the scale above (WARN).
