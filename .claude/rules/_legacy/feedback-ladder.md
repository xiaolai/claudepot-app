---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Feedback Ladder

Every state change in the UI has exactly one canonical feedback surface.
This file picks it deterministically, so long-running or destructive
flows never spray status across four places at once (the blind-spot
Codex named in the rewrite audit).

When you're about to add a new toast, banner, modal, or strip, find the
row in the selection table below. If your state doesn't fit, the state
itself probably needs refactoring — not another surface.

Principle anchor: §5 *One signal per surface* in `design-principles.md`.

## Dimensions

Every state has four dimensions. Score them before picking a surface.

| Dimension | Values |
|---|---|
| **Reversibility** | instant · undoable-in-session · undoable-via-workflow · irreversible |
| **Latency** | instant (<100 ms) · short (<3 s) · long (>3 s, backgrounded) |
| **Scope** | local (this row / this account) · global (whole app / many objects) |
| **Severity** | info · warn · error |

## Selection table

| Scenario | Reversibility | Latency | Scope | Severity | Canonical surface |
|---|---|---|---|---|---|
| Row selection, filter change, sort | instant | instant | local | info | **No feedback** — state is self-evident |
| "Copied to clipboard" | instant | instant | local | info | **Toast (HUD, 2 s)** |
| "Saved" after inline rename | undoable-in-session | short | local | info | **Inline note** next to the field — "Saved just now," fades after 3 s |
| Operation completed (sync done, clean done) | undoable-via-workflow | short | global | info | **Toast (HUD, 4 s)** |
| Operation failed, recoverable | n/a | short | local-or-global | warn | **Inline error on the affected surface** (row, field, panel) |
| Operation failed, unrecoverable, user must read | n/a | short | global | error | **Toast (persistent, error style)** with dismiss + copy-details |
| Long background op with progress (rename, clean, repair) | undoable-via-workflow | long | local-or-global | info | **`RunningOpStrip`** — bottom strip with progress and name |
| Persistent state requiring attention (pending journals, keychain locked) | n/a | — | global | warn | **Banner** at top of the content pane |
| Destructive action about to happen | irreversible | — | local-or-global | warn-or-error | **Modal** with consequence copy; see §3 of `design-principles.md` |
| New content appeared (accounts synced, new project) | instant | — | global | info | **Row appears; no extra feedback** — the list update is the signal |
| Unrecoverable app-level failure (backend dead, DB locked) | n/a | — | global | error | **Banner (bad tone)** plus disable the affected controls |

## Rules

1. **One surface per state.** If you find yourself writing both a toast
   and a banner for the same event, delete the weaker one. Typically the
   toast goes — banners persist, toasts are ephemeral.

2. **Running ops own their status.** A long background op shows in the
   `RunningOpStrip` only. Do *not* also toast "renaming…" while the
   strip is active. When the op completes, the strip disappears and a
   single completion toast (or inline note, for local ops) fires.

3. **Errors escalate, they don't duplicate.** An inline field error
   with a clear recovery path is sufficient. Only escalate to a
   persistent toast if the error crosses surfaces (e.g., backend
   lost) or the user cannot see the origin (background op).

4. **Banners are for state, not events.** "Keychain is locked" is a
   state — banner. "Unlocked successfully" is an event — toast, then
   banner disappears.

5. **Modals are for blocking the user.** If you do not need to block,
   do not use a modal. In particular: modals for reporting completion
   are wrong — use a toast.

6. **No toast for things the user just did deliberately.** "Account
   selected" is a toast only for people who never use computers. The
   selected-state accent is the signal.

## Anti-patterns (name and shame)

- **Status spray.** Event fires a toast, a banner, *and* a log entry
  visible to the user. Pick one.
- **Toast a persistent state.** "Keychain locked" as a 4-second toast —
  the user blinks and misses it, then fails silently.
- **Silent long op.** Background rename with no `RunningOpStrip` entry.
  The user cannot tell whether the app is alive.
- **Modal to celebrate success.** "Rename complete! [OK]" — use a toast.
- **Tooltip carrying the consequence.** Hover-only state for a warning
  that should be visible at rest.
- **Disabled button with no inline reason.** The reason belongs next
  to the button, not in a `title`. (`design-principles.md` §3.)

## Mapping to current Claudepot surfaces

| Component | Correct use | Incorrect use (do not do) |
|---|---|---|
| `ToastContainer` | Ephemeral info/errors, 2-4 s info or persistent error | Long-lived state ("keychain locked"), destructive confirmations |
| `PendingJournalsBanner` | Persistent global state with an action | Event of the moment (e.g., "journal resolved") |
| `RunningOpStrip` | In-flight background ops with progress | Completed ops, synchronous ops |
| `ConfirmDangerousAction` | Irreversible destructive actions | Reversible actions, completion notices |
| Inline row error (`list-row.error`) | Per-row failures, missing credentials | App-level errors |
| `StatusBar` | Constant app-level status (connectivity, sync-age) | Events, transient state |

When adding a new flow, edit this table first. If the flow doesn't map
cleanly, the flow is fighting the ladder — fix the flow.

## Review hook

`design-review.md` Pass B includes a feedback-surface check: for each
new state change in the diff, name the surface chosen and justify it
against this table. If two surfaces fire for the same event, BLOCK.
