---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Design References

Reference apps are the second layer of the design system. The first layer
is `design-principles.md` — open that first, then use this file to pick
the specific thing to imitate.

If a reference conflicts with a principle, the principle wins. Delete or
scope the reference.

References apply to **typography, spacing rhythm, chrome proportion, and
control treatment** — not to the product design. Claudepot is a
developer utility for managing Claude identities and projects; its
interaction model is its own.

## Aesthetic direction

**Operational calm.**

Claudepot is trust-critical. Users are touching keychain secrets, live
tokens, destructive file ops. The tone is the native dev-utility
register: *Keychain Access × Xcode × 1Password 8*, with touches of
Finder for chrome and System Settings for detail-pane rhythm.

Three non-negotiables:

1. **Typography carries hierarchy.** Weight and size do the work, not
   boxes or colored labels.
2. **One accent at a time.** The macOS accent color appears on exactly
   one element at rest (selected item or primary action). No rainbow
   status chips.
3. **Flat, platform-native chrome.** No CSS drop shadows. Vibrancy
   only via Tauri window effects on the nav rail.

## Reference apps

Each reference is a named macOS app. When a design question comes up,
open the reference and look — do not guess. If none of these fit the
situation, you are probably designing something novel and should
escalate to `design-principles.md`.

### Keychain Access — the trust-critical baseline

**Study:** how a native utility that touches credentials communicates
state — the lock icon in the chrome, the explicit "locked / unlocked"
column, the friction on "delete item." Nothing is hidden behind hover.

**Steal:** the idea that security state is persistent UI, not an event.
The visible lock indicator. The way destructive actions land in modals
with explicit consequence copy.

**Don't steal:** Keychain Access's ancient layout (two-pane with
category tabs up top). The structural pattern is outdated; we're
studying its *signaling*, not its *shape*.

### Xcode — the three-pane developer tool

**Study:** left icon rail (Navigator tabs) + list column + detail pane.
Segmented controls with text labels. Dense but legible lists.
Consistent 13 px body. Quiet separators.

**Steal:** nav-rail-to-list-to-detail proportion, segmented controls
that *always* carry text (never icon-only for ambiguous filters), the
restraint on chrome ornamentation.

**Don't steal:** Xcode's toolbar-button density. Claudepot has fewer
actions and should show them more sparingly.

### 1Password 8 (macOS) — the structural twin

**Study:** 48 px icon rail, account switcher at the top of the rail,
categories in the rail, list column in the content pane, detail pane
on the right.

**Steal:** the rail proportions. The way the list-detail split lives
*inside* the content pane, not as a third navigation layer. The detail
pane's airy vertical rhythm.

**Don't steal:** 1Password's colored category icons and favorite
stars. Stay monochromatic.

### Finder — the chrome baseline

**Study:** sidebar vibrancy, traffic lights floating on the sidebar's
vibrant background, the vertical separator that runs uninterrupted
top-to-bottom, zero ornamentation on the content pane.

**Steal:** unified title bar. Traffic lights sit over the rail's
vibrancy, no horizontal separator in the rail's top zone. Rail extends
y=0 to bottom.

**Don't steal:** Finder's toolbar icon density or content layout.

### System Settings (Ventura+) — the detail-pane reference

**Study:** right-aligned label column, left-aligned value column,
generous row spacing, each field a real HIG row with native vertical
rhythm.

**Steal:** the right-aligned label grid in the detail pane. The row
spacing — larger than you'd expect from a web utility.

**Don't steal:** Settings' occasional over-grouping (collapsing every
related field). Claudepot details are flatter.

### Things 3 — narrow-scope typography reference

**Study:** list row typography only — the way a single weight change
separates primary from secondary, the 13/11 px rhythm.

**Steal:** the list-row weight hierarchy. Primary 13 px / 500, meta 11
px / `--muted`.

**Do NOT steal:** Things 3's empty-state warmth, pastel colors,
illustrated affordances. It is a serene personal-productivity app, and
Claudepot is an operational tool. Empty states here should be Keychain
Access-calm, not Things-3-warm.

### Disk Utility — the maintenance-surface reference

**Study:** how a native maintenance tool presents destructive
operations (Erase, Partition) — clear sidebar scope, explicit
consequence copy on the action button itself, progress UI for long
ops.

**Steal:** the way "Erase" opens a sheet that reiterates *what exactly*
will disappear. Claudepot's Clean and Rename flows should feel the
same.

**Don't steal:** Disk Utility's dated layout. Study the flow, not the
frame.

## Anti-references

If any of these aesthetics creep in, back up:

- **Generic Tailwind SaaS dashboard.** Stripe-clone, GitHub-settings-
  clone, shadowed cards on a gray wash.
- **Electron web-wrapper apps** (Slack, Discord, Notion desktop).
  Foreign on macOS.
- **Bootstrap/Material defaults.** Bright buttons, elevated cards.
- **"AI-generated UI" pattern.** Purple gradient on white, Inter
  everywhere, all-caps tracking on every heading, emoji icons,
  rounded-2xl on rows.
- **Things 3 *everywhere***. See above — we steal its typography and
  nothing else.

## Taste test

Replace the old "would this look out of place in Finder's toolbar?" —
that test biased toward restraint in a trust-critical utility. Use
this one instead:

> **The Five-Second Test.**
>
> A cautious developer opens this view for the first time. In five
> seconds, can they say out loud:
>
> 1. What object is selected?
> 2. What state is it in?
> 3. What will happen if they click the biggest button?

If any answer is "I'm not sure," the design isn't done. This is the
same test codified in `design-principles.md` §8.

Apply the test before opening `design-patterns.md` for recipes. A
recipe that passes the test in isolation can still fail in context
(e.g., a second primary button on the page makes the answer to Q3
ambiguous).
