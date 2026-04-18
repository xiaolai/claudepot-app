---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Design Principles

This is the top layer of the Claudepot design system. References, patterns,
and tokens all derive from these principles — they are not a separate
layer of obedience, they are the *why*.

When a rule here conflicts with a recipe in `design-patterns.md` or a
token in `ui-design-system.md`, the principle wins. Rewrite the recipe.

## 1. Implementation detail never outranks user identity

The primary surface of any detail view is what a human would ask for.
Emails, project names, dates, sizes, counts. Never DB keys, sanitized
slugs, UUIDs, internal paths, or any identifier the user did not
choose.

- If an internal identifier is actually useful (for debugging or
  support), put it behind a disclosure, a context-menu "Copy ID," or
  the detail pane's footer — never in the primary grid.
- If an identifier is never useful to the user, delete it from the
  view entirely. Tests should catch that it is gone.

**Current failure:** the `Key: -Users-joker-github-xiaolai-myprojects-tepub`
row in the project detail pane. This slug is a DB key, not a project.

## 2. Selected object is unmistakable

One element on the screen is the current subject. A cold user should be
able to tell which one in under a second.

- Exactly one accent-colored item visible at rest.
- Contrast is structural — background fill + weight change — not just
  border or color alone.
- The selected state must survive `prefers-contrast: more` and still
  read as selected. Accent-weak backgrounds fail this; add a weight or
  icon change.

## 3. Destructive actions state consequence inline

"Delete," "clean," "reset," "rollback" are not styling choices. The
button must say what will happen, to how many things, and whether it
can be undone — before the confirmation modal appears, not inside it.

- Button label: include the object and, when finite, the count.
  `Clean 14 projects` beats `Clean…`.
- Inline hint text: one line under or next to the button,
  `Removes 32.4 MB of session data. Cannot be undone.` Not a tooltip
  — tooltips do not exist on macOS for mouse users without hover.
- Confirmation modals repeat the consequence in the primary verb.
  Not `OK`. `Delete 14 projects`.

**Current failure:** the disabled `Clean…` button with no inline reason
and no consequence copy.

## 4. State legibility beats chrome restraint

Claudepot is trust-critical. Users are touching keychain secrets, live
credentials, destructive file ops. When a state needs explaining, an
explicit surface (banner, inline note, persistent strip) is correct —
even if it costs visual quiet.

- Never hide a consequential state behind hover or a tiny icon.
- Never flash it in a 4-second toast for a state that persists longer.
- If the chrome feels "too loud," question the state itself — maybe
  the state is the problem, not the banner.

This principle outranks the aesthetic direction in `design-references.md`.
Prefer clarity.

## 5. One signal per surface

Every state change needs a single canonical feedback surface, not
three echoes. See `feedback-ladder.md` for the mapping.

- Do not show the same event as a toast *and* a banner *and* a
  running-op strip *and* an inline note. Pick the right surface
  per the ladder and commit.
- Exception: logging the event to stderr or an audit log is not a
  user-facing signal and does not count.

**Current failure (latent):** long-running ops have both `RunningOpStrip`
and toasts; rename completion fires both. Choose one.

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
  persistent chrome, not a settings panel.
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

If any of those is unclear, the design is not done. This replaces any
weaker "does it look native" test.

---

## How these principles interact with the rest of the system

```
design-principles.md         ← WHY (this file)
    │
    ├── design-references.md ← WHAT to imitate (scoped to typography, chrome, proportions)
    ├── feedback-ladder.md   ← WHEN to use which surface
    ├── design-patterns.md   ← HOW to compose a specific element
    └── ui-design-system.md  ← WHICH tokens and current defaults
```

`design-review.md` checks every PR against the principles *first*, then
patterns, then tokens. Principle violation is always BLOCK.
