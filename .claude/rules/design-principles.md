# Design Principles — paper-mono

This is the top layer of the Claudepot design system. Tokens, patterns,
and references all derive from these principles — they are not a separate
layer of obedience, they are the *why*.

When a rule here conflicts with a recipe in `design-patterns.md` or a
token in `ui-design-system.md`, the principle wins. Rewrite the recipe.

## Register: paper-mono, dev-dialect

Claudepot is a Tauri desktop utility for managing Claude identities,
projects, and credentials. The register is **paper-mono**:

- **One typeface, every glyph.** JetBrains Mono Nerd Font — body text,
  chrome, and iconography. No proportional fallback, no secondary
  family, no SVG icon library.
- **Warm paper surfaces.** Light mode is warm paper-white; dark mode
  is ink paper. OKLCH palette with low chroma for neutrals and a
  single terracotta accent.
- **Small radii, hairline borders.** Mono typography reads flat;
  large rounding and heavy shadows fight the aesthetic. Max 10px
  radius, 1px borders.
- **Dev-dialect, not native-mimic.** We do not pretend to be an
  AppKit app. Precedents: Warp, Zed, Lapce, Linear, Arc. The custom
  `WindowChrome` breadcrumb bar is deliberate — it carries the
  `⌘K` surface and the theme toggle.

## 1. Implementation detail never outranks user identity

The primary surface of any detail view is what a human would ask for.
Emails, project names, dates, sizes, counts. Never DB keys, sanitized
slugs, UUIDs, internal paths, or any identifier the user did not
choose.

- If an internal identifier is useful for debugging, put it behind a
  disclosure, a context-menu "Copy ID," or a `DevBadge` that only
  appears when Developer mode is on.
- If an identifier is never useful to the user, delete it from the
  view entirely. Tests should catch that it is gone.

## 2. Selected object is unmistakable

One element on the screen is the current subject. A cold user should
be able to tell which one in under a second.

- Exactly one accent-colored item visible at rest.
- Contrast is structural — left-border 2–3px accent + background fill
  + weight change — not just color alone.
- The selected state must survive dark mode and `prefers-contrast:
  more` and still read as selected.

## 3. Destructive actions state consequence inline

"Delete," "clean," "reset," "rollback" are not styling choices. The
button must say what will happen, to how many things, and whether it
can be undone — before the confirmation modal appears, not inside it.

- Button label: include the object and, when finite, the count.
  `Clean 14 projects` beats `Clean…`.
- Inline hint text under or next to the button. No tooltip-only
  disclosures.
- Confirmation modals repeat the consequence in the primary verb.
  Not `OK`. `Delete 14 projects`.

## 4. State legibility beats chrome restraint

Claudepot is trust-critical. Users are touching keychain secrets, live
credentials, destructive file ops. When a state needs explaining, an
explicit surface (banner, inline note, persistent strip) is correct —
even if it costs visual quiet.

- Never hide a consequential state behind hover or a tiny icon.
- Never flash it in a 4-second toast for a state that persists longer.
- If the chrome feels "too loud," question the state itself — maybe
  the state is the problem, not the banner.

## 5. One signal per surface

Every state change needs a single canonical feedback surface, not
three echoes. Every new state surface maps to exactly one of: toast,
banner, inline note, `RunningOpStrip`, modal. See `design-patterns.md`
for the selection table.

- Do not show the same event as a toast *and* a banner *and* a
  running-op strip *and* an inline note. Pick one and commit.
- Logging the event to stderr or an audit log is not a user-facing
  signal and does not count.

## 6. Reversibility shapes friction

The amount of friction between intent and irreversible action must
match the blast radius.

| Reversibility | Friction |
|---|---|
| Instant, local, undoable (select, filter, sort) | None |
| Undoable within session (rename, move) | One click, no modal |
| Undoable via workflow (repair journal) | Confirmation, clear rollback path stated |
| Irreversible + local (delete one project) | Modal with typed confirmation or count echo |
| Irreversible + global (remove all accounts, clean everything) | Modal + consequence copy + primary button labeled with the verb |

Never add friction where none is warranted (e.g., confirming a sort).
Never skip friction where it is (e.g., a naked "Delete all" button).

## 7. Keychain and credential state are first-class UI

The user cannot trust an app that hides whether it is locked, whether
its credentials are valid, or whether its last sync succeeded.

- Keychain lock state, credential freshness, last-sync-age belong in
  persistent chrome (the bottom `StatusBar` sync dot, the card
  `AnomalyBanner`), not a settings panel.
- When a credential is expired, stale, or missing, the affected
  surface says so inline — not only in the error path.
- Never log, toast, or display a full credential. Truncation rules
  live in `rust-conventions.md` and apply to the frontend too.

## 8. Test the first five seconds

Any new view should pass this reading: a cautious developer opens it
for the first time and within five seconds can say out loud:

1. What object is selected?
2. What state is it in?
3. What will happen if they click the biggest button?

If any of those is unclear, the design is not done.

## 9. Dark-first parity

Every design decision must be tested in both light and dark and
neither may feel like an afterthought. Our OKLCH palette is authored
so dark mode mirrors light via the same semantic variable names
(`--bg`, `--fg`, `--line`, `--accent`) — changing the theme attribute
on `<html>` flips the palette without any component code change.

- Every new color must be a variable, not a raw hex/rgb/oklch literal
  in a component.
- Prefer monochromatic-plus-one-accent palettes. Rainbow state colors
  make dark-mode parity harder and usually indicate missing
  hierarchy.
- `Tag` tones: `neutral | accent | ok | warn | danger | ghost`. No
  other color roles.

## 10. No emoji, no icon library

Every glyph is a Nerd Font codepoint drawn via the `Glyph` primitive
(`src/components/primitives/Glyph.tsx`), looked up in the NF map in
`src/icons.ts`. No lucide, heroicons, Font Awesome SVGs, or emoji in
UI. If you need a new icon, find the NF codepoint on the Nerd Fonts
cheat sheet and add it to the map.

This is what gives the app its visual identity. A single SVG icon
mixed in breaks the aesthetic instantly.

---

## Layer interaction

```
design-principles.md         ← WHY (this file)
    │
    └── design-patterns.md   ← HOW to compose a specific element
         │
         └── ui-design-system.md  ← WHICH tokens and current defaults
              │
              └── accessibility.md  ← a11y floor (not the ceiling)
```

`design-review.md` (if present) checks PRs against the principles
*first*, then patterns, then tokens. Principle violation is always
BLOCK.

## Legacy

Previous native-macOS design rules are archived under
`.claude/rules/_legacy/`. They describe a superseded aesthetic (`-apple-system-*`
tokens, native chrome, 48px icon rail). Do not consult them for new
work. They remain for archaeology only.
