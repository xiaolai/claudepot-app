# UI Design System — tokens and defaults (paper-mono)

All tokens live in `src/styles/tokens.css`. This file is the
authoritative reference for what each token means and where it came
from. Every value below is a CSS variable — do not re-declare colors,
radii, sizes, or spacings in components.

## Load order

| Layer | File | Authority |
|---|---|---|
| Why | `design-principles.md` | Non-negotiable. Wins conflicts. |
| How | `design-patterns.md` | Composition recipes. |
| Which tokens | this file | Current defaults. |
| A11y floor | `accessibility.md` | Not the ceiling. |
| Code conventions | `react-components.md` | Component file shape. |

## Tokens — color

Every color is an OKLCH value. Dark mode redefines the same semantic
variables on `[data-theme="dark"]`, so component CSS never references
a light-only or dark-only token.

### Accent (Claude terracotta)

| Var | Light | Dark |
|---|---|---|
| `--accent`        | `oklch(68% 0.13 45)`          | `oklch(74% 0.13 45)` |
| `--accent-soft`   | `oklch(68% 0.13 45 / 0.12)`   | `oklch(74% 0.13 45 / 0.16)` |
| `--accent-border` | `oklch(68% 0.13 45 / 0.28)`   | `oklch(74% 0.13 45 / 0.35)` |
| `--accent-ink`    | `oklch(42% 0.10 45)`          | `oklch(80% 0.12 45)` |

### Semantic (used sparingly)

| Var | Value | Role |
|---|---|---|
| `--ok`      | `oklch(62% 0.10 150)` | Success only |
| `--warn`    | `oklch(72% 0.12 80)`  | Warning only |
| `--danger`  | `oklch(60% 0.14 25)`  | Destructive only |

### Neutrals — light (warm paper)

| Var | Value |
|---|---|
| `--bg`          | `oklch(99% 0.003 60)` |
| `--bg-raised`   | `oklch(100% 0 0)` |
| `--bg-sunken`   | `oklch(97% 0.004 60)` |
| `--bg-hover`    | `oklch(95% 0.005 60)` |
| `--bg-active`   | `oklch(93% 0.008 60)` |
| `--fg`          | `oklch(22% 0.008 60)` |
| `--fg-muted`    | `oklch(50% 0.008 60)` |
| `--fg-faint`    | `oklch(65% 0.006 60)` |
| `--fg-ghost`    | `oklch(78% 0.004 60)` |
| `--line`        | `oklch(92% 0.005 60)` |
| `--line-strong` | `oklch(85% 0.006 60)` |
| `--scrim`       | `oklch(30% 0.01 60 / 0.28)` |

### Neutrals — dark (ink paper)

| Var | Value |
|---|---|
| `--bg`          | `oklch(16% 0.006 60)` |
| `--bg-raised`   | `oklch(19% 0.006 60)` |
| `--bg-sunken`   | `oklch(13% 0.005 60)` |
| `--bg-hover`    | `oklch(23% 0.007 60)` |
| `--bg-active`   | `oklch(27% 0.008 60)` |
| `--fg`          | `oklch(92% 0.006 60)` |
| `--fg-muted`    | `oklch(70% 0.008 60)` |
| `--fg-faint`    | `oklch(55% 0.008 60)` |
| `--fg-ghost`    | `oklch(40% 0.006 60)` |
| `--line`        | `oklch(26% 0.008 60)` |
| `--line-strong` | `oklch(34% 0.010 60)` |
| `--scrim`       | `oklch(0% 0 0 / 0.55)` |

### Shadows

`--shadow-sm` hairline drop · `--shadow-md` `0 2px 8px rgba(0,0,0,.04)`
(light) / `.30` (dark). Modals use custom larger shadows inline.

Rule: list rows, detail rows, and chrome surfaces carry no shadow.
Elevation is reserved for popovers, dropdowns, and modals.

## Tokens — spacing (4px base)

`--s-0: 0 · --s-1: 4 · --s-2: 8 · --s-3: 12 · --s-4: 16 · --s-5: 20
· --s-6: 24 · --s-8: 32 · --s-10: 40 · --s-12: 48 · --s-16: 64`

House style. Invented spacings outside this scale are a review
finding.

## Tokens — typography

Font family (one, everywhere):

```css
--font: 'JetBrainsMonoNF', 'JetBrains Mono', ui-monospace, Menlo, monospace;
```

The `NF` suffix is the Nerd Font variant — every glyph, including the
~9k icon codepoints, is forced to the monospace cell so lists and
columns align perfectly. Files live in `src/assets/fonts/` at weights
400 / 500 / 600 / 700.

### Size scale

| Var | Size | Use |
|---|---|---|
| `--fs-xs`   | 11 px | statusbar, meta, mono-caps labels |
| `--fs-sm`   | 12 px | dense body, button text |
| `--fs-base` | 13 px | default body, list rows |
| `--fs-md`   | 15 px | emphasis, card titles |
| `--fs-lg`   | 18 px | section titles |
| `--fs-xl`   | 22 px | screen titles |
| `--fs-2xl`  | 28 px | hero |

Seven sizes; invented sizes outside this scale are a review finding.

### Line heights

`--lh-tight: 1.25 · --lh-body: 1.5 · --lh-loose: 1.7`

### Letter spacing

`--ls-tight: -0.01em` (screen titles) · `--ls-normal: 0` (default) ·
`--ls-wide: 0.08em` (mono-caps) · `--ls-wider: 0.14em` (breadcrumb,
status-bar labels).

### Base applied to `<body>`

```css
-webkit-font-smoothing: antialiased;
-moz-osx-font-smoothing: grayscale;
font-feature-settings: "calt" 1, "liga" 1;
font-variant-numeric: tabular-nums;
```

Global `tabular-nums` means every number in the UI lines up in
columns by default — no per-class opt-in needed.

## Tokens — radius

`--r-0: 0 · --r-1: 3px · --r-2: 6px · --r-3: 10px · --r-pill: 999px`

**Keep radii small.** Mono type reads flat; large rounding clashes
with the aesthetic. Buttons, inputs, rows: `--r-2`. Modals, cards:
`--r-3`. Badges: `--r-pill` or `--r-1`.

## Utility classes (tokens.css)

- `.mono-cap` — uppercase 11px letter-spaced-wider `--fg-muted` label
- `.mono-faint` — `--fg-faint`
- `.mono-muted` — `--fg-muted`
- `.mono-accent` — `--accent-ink`
- `.nf` — Nerd Font face + normal feature-settings (icon glyphs)

## Button variants

Five canonical variants, modeled on shadcn/ui so the taxonomy is
portable:

| Variant | Visual | When |
|---|---|---|
| `solid`   | Accent fill, white text | One primary action per view |
| `ghost`   | Transparent at rest, `--bg-hover` on hover | Toolbar, sidebar nav |
| `subtle`  | `--bg-sunken` fill, hairline border | Secondary action |
| `outline` | Transparent fill, `--line-strong` border | Tertiary action |
| `accent`  | Transparent fill, `--accent-border`, `--accent-ink` text | Mild accent (copy-link style) |

Danger: apply `danger` modifier that overrides color to `--danger`.

Height scale: `sm` 24 px · `md` 30 px (default) · `lg` 36 px.

Rule: exactly **one** `solid` button visible per view. Two means you
haven't decided which action matters.

## Icons — Nerd Font only

All iconography is Nerd Font codepoints, centralized in
`src/icons.ts`. Render via the `Glyph` primitive
(`src/components/primitives/Glyph.tsx`). No lucide, heroicons,
Font Awesome SVGs, or emoji. This is a BLOCK-level rule.

Size hints:

| Context | Size |
|---|---|
| Sidebar nav | 14 px |
| Toolbar / inline | 13 px |
| Button | `currentColor` at 1.2em |
| Status bar | 10 px |
| Empty-state hero | 28–32 px |

## Cursor and selection

`cursor: pointer` is allowed on interactive buttons in paper-mono
(reverses the previous native-macOS rule) — the aesthetic is
dev-utility, not Finder-clone. `user-select: none` on chrome;
only content text (paths, emails, tokens, `.selectable`) is
selectable.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| ⌘K / Ctrl+K | Open command palette |
| ⌘R | Refresh |
| ⌘N | Add account |
| ⌘, | Settings |
| ⌘1/2/3/4 | Switch primary nav section |
| ⌘F | Focus search |
| Escape | Close modal / palette |

Never fire while a modal is open or an input has focus. Standard
macOS Cmd+Q/W/H/M come free from Tauri.

## Theme

- Toggled top-right in `WindowChrome`; persisted in
  `localStorage['cp-theme']`.
- Sets `document.documentElement.dataset.theme = 'light' | 'dark'`.
- Defaults to `prefers-color-scheme` on first run.
- All color changes flow through the tokens above — no per-component
  theme code.

## Developer mode

- Toggled in Settings → General; persisted in
  `localStorage['cp-dev-mode']`.
- When on, `<DevBadge>` components surface backend command names,
  raw paths, and UUIDs next to their human-facing labels.
- Changes fire a `cp-dev-mode-change` window event so every mounted
  `<DevBadge>` updates live.

## Legacy token aliases

During the port, `src/styles/tokens.css` also defines the previous
rule system's names (`--surface`, `--border`, `--text`, `--muted`,
`--chrome`, `--hover-bg`, `--accent-weak`, `--ok-weak`, `--bad`,
`--bad-weak`, `--warn-weak`, `--focus-ring`, `--tertiary`,
`--accent-text`, `--dur-fast`, `--dur-base`, `--dur-slow`,
`--ease-out`, `--ease-in-out`, `--ease-in`, `--shadow-0..3`) so
unported components keep rendering without a colour rewrite.

As components are ported to the paper-mono primitives, drop their
legacy-alias use. When the last referencing component is migrated,
delete the alias. Aliases are **not** a supported surface for new
code.
