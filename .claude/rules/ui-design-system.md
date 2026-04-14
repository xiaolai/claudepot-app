---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# UI Design System

## Colors

All colors via CSS custom properties. Never use raw hex, rgb(), or hsl() in
component styles.

Light + dark tokens defined in `src/App.css :root` and
`@media (prefers-color-scheme: dark)`. Every new color token must have both
a light and dark variant.

Semantic tokens (use these, not raw values):

| Token | Purpose |
|-------|---------|
| `--bg` | Page background |
| `--surface` | Card/modal/toast background |
| `--border` | Subtle borders |
| `--border-strong` | Button/input borders |
| `--text` | Primary text |
| `--muted` | Secondary/helper text |
| `--accent` | Primary action, active state |
| `--accent-weak` | Active state background |
| `--ok` / `--ok-weak` | Healthy/valid status |
| `--bad` / `--bad-weak` | Error/danger status |
| `--warn` / `--warn-weak` | Warning status |
| `--shadow` | Box shadow for elevated surfaces |

Do not add new tokens without updating both light and dark definitions.

## Spacing

4px grid. Use only these values for margin, padding, and gap:

`4px · 6px · 8px · 10px · 12px · 14px · 16px · 20px · 24px · 32px · 48px · 60px`

## Typography

Font sizes (in px): `11 · 12 · 13 · 14 · 15 · 16 · 18 · 22`

- 11px: badges, uppercase labels
- 12px: meta text, mono code, helper text
- 13px: toast text
- 14px: body (base)
- 15px: card headings
- 16px: modal headings
- 18px: empty state headings
- 22px: app title

Font weights: 400 (body), 500 (buttons, pill values), 600 (headings, badges, labels)

Monospace: `"SF Mono", Menlo, Consolas, monospace`

## Border radius

| Element | Radius |
|---------|--------|
| Buttons | 7px |
| Cards, banners, empty state | 10px |
| Modals | 12px |
| Pills, badges | 999px |

## Shadows

Only use `var(--shadow)` for card-level elevation.
Modals use a hardcoded deep shadow: `0 20px 60px rgba(0, 0, 0, 0.25)`.
Toasts use: `0 8px 24px rgba(0, 0, 0, 0.15)`.

## Transitions

Interactive elements: `0.12s ease` for `background`, `border-color`, `filter`.
Do not animate layout properties (width, height, margin, padding).

## Dark mode

Every visual change must be tested in both light and dark mode.
Use `prefers-color-scheme: dark` media query — no manual toggle.
Never use absolute colors (e.g., `#ffffff` for text) — always tokens.
Exception: `#fff` is acceptable for text on `--accent` or `--bad` primary buttons.
