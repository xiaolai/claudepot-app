---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# UI Design System — macOS Native

All values pinned to AppKit/SwiftUI defaults (macOS Sonoma/Sequoia).
Full reference: `dev-docs/macos-native-design-system.md`.

## Principles

1. System colors over hardcoded hex. Use `-apple-system-*` tokens.
2. 13px base font (macOS native). Not 14px, not 16px.
3. `cursor: default` everywhere. Pointer cursor is for hyperlinks only.
4. `user-select: none` on UI chrome. Only content text (`.selectable`).
5. Invisible borders at rest. Buttons show background fill on hover only.
6. No box shadows on list items. Flat 0.5px separators.
7. Vibrancy via Tauri `windowEffects`, not CSS backdrop-filter hacks.

## Colors

All colors via CSS custom properties backed by `-apple-system-*` tokens.
Auto-adapt to light/dark, user accent color, and accessibility settings.

| Token | Source | Purpose |
|-------|--------|---------|
| `--bg` | `-apple-system-background` | Content background |
| `--surface` | `-apple-system-secondary-background` | Modal/card background |
| `--border` | `-apple-system-separator` | Borders (0.5px) |
| `--text` | `-apple-system-label` | Primary text |
| `--muted` | `-apple-system-secondary-label` | Secondary text |
| `--accent` | `AccentColor` | User's system accent |
| `--accent-weak` | accent at 12-18% opacity | Active state bg |
| `--ok` / `--ok-weak` | `-apple-system-green` | Success |
| `--bad` / `--bad-weak` | `-apple-system-red` | Error |
| `--warn` / `--warn-weak` | `-apple-system-orange` | Warning |
| `--focus-ring` | 3px accent at 30% | macOS focus ring |

`color-scheme: light dark` on `:root`. Dark values auto-adjusted via
`@media (prefers-color-scheme: dark)`.

## Icons

`@phosphor-icons/react`, `light` weight, 16px default.
Never mix icon libraries. All icons come from Phosphor.

| Context | Weight | Size |
|---------|--------|------|
| Toolbar / sidebar | `light` | 16px |
| Inline / buttons | `light` | 14px |
| Active state | `fill` | same |
| Copy indicators | `bold` | 13px |
| Empty state | `thin` | 32px |

## Typography

Font: `-apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif`
Mono: `"SF Mono", SFMono-Regular, ui-monospace, Menlo, monospace`

| Element | Size | Weight |
|---------|------|--------|
| Body text | 13px | 400 |
| Sidebar section header | 11px | 600, uppercase, 0.5px tracking |
| Button label | 13px | 400 (500 for primary) |
| Detail heading (h2) | 16px | 600 |
| Section title (h3) | 11px | 600, uppercase |
| Badge / tag | 10px | 600 |
| Monospace | 11px | 400 |
| Sidebar item meta | 11px | 400 |
| Modal heading | 13px | 700 |

## Spacing

4px grid: `2 · 4 · 6 · 8 · 10 · 12 · 16 · 20 · 24 · 32`

## Border Radius

| Element | Radius |
|---------|--------|
| Buttons, inputs, sidebar items | 6px |
| Banners | 8px |
| Toasts | 10px |
| Modals | 12px |
| Pills, badges | 999px |

## Layout

Sidebar (240px, transparent) + Content pane (opaque `var(--bg)`).
52px top padding clears the overlay title bar / traffic lights.

## Buttons

Height 28px. Border 0.5px var(--border). Border-radius 6px.
Hover = background fill, never border change.
Focus = `var(--focus-ring)`.
Icon-only = `.icon-btn` (28x28, no border, no shadow).

## Modals

Width 440px. Backdrop rgba(0,0,0,0.30). Radius 12px.
Button order: Cancel (left), Confirm (right) — macOS convention.

## Toasts

Top-center HUD style. Dark translucent with backdrop-filter blur.
Error toasts = red bg. All toasts have white text.

## Anti-Patterns (never do these)

- `cursor: pointer` on buttons
- `::-webkit-scrollbar` overrides
- Box shadows on list items
- `border-radius > 8px` on non-modal elements
- Visible button borders at rest
- Modal backdrop > 0.30 opacity
- 14px or 16px body font
- Corner-positioned toasts
- Hardcoded hex colors (use tokens)
