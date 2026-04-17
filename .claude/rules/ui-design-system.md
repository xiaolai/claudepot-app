---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# UI Design System — macOS Native

All values pinned to AppKit/SwiftUI defaults (macOS Sonoma/Sequoia/Tahoe).
Full reference: `dev-docs/macos-native-design-system.md`.

## Principles

1. System colors over hardcoded hex. Use `-apple-system-*` tokens.
2. 13px base font (macOS native). Not 14px, not 16px.
3. `cursor: default` everywhere. Pointer cursor is for hyperlinks only.
4. `user-select: none` on UI chrome. Only content text (`.selectable`).
5. Invisible borders at rest. Buttons show background fill on hover only.
6. No box shadows on list items. Flat 0.5px separators.
7. Vibrancy via Tauri `windowEffects`, not CSS backdrop-filter hacks.
8. Respect all accessibility media queries: `prefers-reduced-motion`,
   `prefers-contrast`, `prefers-reduced-transparency`.
9. Context menus on every interactive object — macOS users expect them.
10. Standard keyboard shortcuts — Cmd+R, Cmd+N, Cmd+, at minimum.

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

`lucide-react`, stroke-based, 16px default via CSS (`svg.lucide`).
Never mix icon libraries. All icons come from Lucide.
No colored icons except status indicators (dots, badges).

| Context | strokeWidth | Size |
|---------|-------------|------|
| Toolbar / sidebar | 1.5 (CSS default) | 16px |
| Inline / buttons | 1.5 | 14px |
| Active state indicator | 2.5 | same |
| Copy/toast emphasis | 2.5 | 13-14px |
| Empty state | 1 | 28-32px |

## Typography

Font: `-apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif`
Mono: `"SF Mono", SFMono-Regular, ui-monospace, Menlo, monospace`

### macOS text styles (NSFont.TextStyle reference)

| HIG Style | Size | Weight | Claudepot usage |
|-----------|------|--------|-----------------|
| Large Title | 26px | 400 | — (reserved) |
| Title 1 | 22px | 400 | — (reserved) |
| Title 2 | 17px | 400 | — (reserved) |
| Title 3 | 15px | 400 | Detail heading (h2) |
| Headline | 13px | 600 | Selected sidebar item |
| Body | 13px | 400 | Body text, buttons |
| Callout | 12px | 400 | — (reserved) |
| Subheadline | 11px | 400 | Sidebar meta, section headers |
| Footnote | 10px | 400 | — (reserved) |
| Caption 1 | 10px | 500 | Badges, tags |
| Caption 2 | 10px | 400 | — (reserved) |

### Claudepot element scale

| Element | Size | Weight | HIG mapping |
|---------|------|--------|-------------|
| Body text | 13px | 400 | Body |
| Sidebar section header | 11px | 600, uppercase, 0.5px tracking | Subheadline (bold) |
| Button label | 13px | 400 (500 for primary) | Body |
| Detail heading (h2) | 15px | 600 | Title 3 |
| Section title (h3) | 11px | 600, uppercase | Subheadline (bold) |
| Badge / tag | 10px | 600 | Caption 1 |
| Monospace | 11px | 400 | Subheadline (mono) |
| Sidebar item meta | 11px | 400 | Subheadline |
| Modal heading | 13px | 700 | Headline |

Note: detail heading was 16px, corrected to 15px to match HIG Title 3.

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

Note: 28px is a conscious deviation from AppKit's regular control
height (22pt). Web UIs need slightly larger targets; 28px matches
Slack, VS Code, and 1Password on macOS.

## Modals

Width 440px. Backdrop rgba(0,0,0,0.30). Radius 12px.
Button order: Cancel (left), Confirm (right) — macOS convention.

## Toasts

Top-center HUD style. Dark translucent with backdrop-filter blur.
Error toasts = red bg. All toasts have white text.

## Context Menus

macOS users expect right-click context menus on every interactive object.
Use `onContextMenu` handler — never suppress the event without providing
a custom menu.

Required context menus:
- Account cards: Copy email, Copy UUID, Set as CLI, Set as Desktop, Remove
- Project rows: Open in Finder, Rename, Clean, Copy path
- Token badges: Copy token (truncated), Refresh
- Selectable text: system default (Copy)

## Keyboard Shortcuts

Standard macOS shortcuts must work. Tauri provides Cmd+Q/W/H/M by default.
The app must wire these additional shortcuts:

| Shortcut | Action |
|----------|--------|
| Cmd+R | Refresh accounts / projects |
| Cmd+N | Add account |
| Cmd+, | Open settings (when settings view exists) |
| Cmd+1/2/… | Switch sections in the rail |
| Cmd+F | Focus filter/search (when search exists) |
| Escape | Close modal / deselect |

## Accessibility Media Queries

### `prefers-contrast: more`

When Increase Contrast is enabled (System Settings > Accessibility >
Display), borders and separators must become more prominent:
```css
@media (prefers-contrast: more) {
  :root { --border: rgba(0, 0, 0, 0.30); }
}
@media (prefers-contrast: more) and (prefers-color-scheme: dark) {
  :root { --border: rgba(255, 255, 255, 0.30); }
}
```

### `prefers-reduced-transparency`

When Reduce Transparency is enabled, the sidebar must have an opaque
background instead of relying on vibrancy:
```css
@media (prefers-reduced-transparency) {
  .sidebar { background: var(--bg); }
}
```

## macOS Tahoe / Liquid Glass (forward-looking)

macOS Tahoe (2025) introduced Liquid Glass. Key implications:
- Toolbars and sidebars get glass automatically in native apps.
- Explicit `NSVisualEffectView` in sidebars **blocks** glass — must be
  removed when targeting Tahoe.
- Tauri's `windowEffects: ["sidebar"]` may need removal once Tauri
  adds Liquid Glass support.
- CSS `background: transparent` on sidebar continues to work.
- New control sizes: slightly taller mini/small/medium, new extra-large.
- No glass-on-glass stacking — single `GlassEffectContainer` groups
  multiple glass elements.
- Accessibility is automatic: Reduce Transparency = frostier glass,
  Increase Contrast = opaque with border.

Do not refactor for Tahoe yet. Track Tauri 2 release notes for glass API.

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
- Missing context menus on interactive objects
- Suppressing right-click without providing a custom menu
- Hardcoded font sizes outside the HIG text styles table
- Missing `prefers-contrast` / `prefers-reduced-transparency` handling
