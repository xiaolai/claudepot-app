# The test gates — what each one exists to catch

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries every gate here was written after a specific failure shipped green; this is the record of which one.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## Test

```bash
cargo test --workspace               # Rust
cargo xtask verify-cc-parity         # CC settings-merge parity goldens (see parity-harness/README.md)
pnpm test                            # React (Vitest + RTL, jsdom)
pnpm test:coverage                   # React with coverage report
cd panel && pnpm check:render        # the built remote panel actually mounts
pnpm check:classes                   # every className has a CSS rule behind it
pnpm check:a11y                      # every role="switch" has an accessible name
```

`panel`'s render check answers a question `vite build` cannot: whether
the bundle *mounts*. It runs the committed output in jsdom and asserts
**seven** passes, because for a while it only asserted the first:

- **signed out**, with no network — the offline path a phone hits first
  — reaching the sign-in screen with zero console errors;
- **signed in**, against a stub host, opening a session and reaching the
  thread's composer — and asserting that opening it pushed a history
  entry, because the back-gesture feature guards itself and fails off,
  so without that line the pass would stay green while exercising
  nothing;
- the **quick-prompt sheet**, which is the shared `PickerSheet` chrome;
- the **slash-command sheet**, which is a different fetch, a different
  row, and an argument step the other picker has no equivalent of;
- **staging** that command — a distinct end state, since `stage()` closes
  the sheet, so "sheet open with an args field" and "sheet closed with a
  chip in the composer" cannot be asserted in one pass;
- the **offline queue**, as a round trip: cut the wire, send, assert the
  message was HELD and never reached the host, restore the wire, assert
  it went out under the entry's own idempotency key;
- the **wide two-pane layout**, declared at 1200px.

Everything after the first exists because vite does not resolve free
identifiers, so a missing import is a runtime `ReferenceError` in
whatever code path touches it. Two of them shipped in one commit —
`useEffect`, then `api` — and turned every thread into a blank screen
while the signed-out assertion stayed green, since it never reaches
`Thread`. Reverting either import now fails the check; verified in both
directions.

The offline pass is the only end-to-end coverage the drain has — the
store has unit tests, the loop that reads it had none — and it is the
one path in the panel that sends a message the user is not present for.
Watched failing against a drain that minted a fresh key instead of
replaying the entry's.

**A `className` with no rule renders as unstyled markup, and
`pnpm check:classes` is the only thing that says so.** It is valid HTML,
invisible to `tsc`, and invisible to a render test that asserts on text
— so `RemotePane` shipped against eight invented class names (`pane`,
`pane-block`, `pane-intro`, `pane-warning`, `pane-error`,
`pane-actions`, `remote-devices`, `status-chip`) with every other gate
green.

It was the *second* instance, which is why the answer is a gate rather
than a third careful reading: `QuickPromptsPane` had been rendering a
dead `pane` since it was written, and `ProtectedPathsPane` carries a
comment from an earlier pass that found `className="btn outline"` doing
nothing. The scan found eight more across the renderer, all now removed.

Three details it needs to be honest:

- **Comments are stripped first.** `ProtectedPathsPane` quotes the dead
  `className="btn outline"` inside the comment explaining its removal,
  and the first version reported that as a live finding.
- **It refuses a vacuous pass.** An empty corpus on either side reports
  zero orphans, so it fails when it finds fewer than 100 of either.
- **`lucide*` is exempt** — `lucide-react` stamps
  `class="lucide lucide-<name>"` onto every icon SVG, and those belong to
  the library. Scoped to that prefix so it cannot become a general
  escape hatch.

A class that exists only to be queried by a test is a `data-testid`, not
a class — `MarkdownRenderer`'s `md-link` was the one such case and now
says so.

**The same script's second half asks whether a text field draws chrome
at all.** `tokens.css` gives `input, textarea` only `font` and `color` —
no border reset, no background, no radius — so a bare one renders with
the user-agent border, which in WebKit is a 2px INSET bevel on an input
and a 1px grey rule on a textarea. `QuickPromptsPane` had one of each,
six inches from fields that went through `Input`. The panel hit this
independently; `panel/src/controls.css` records the same measurement.

The fix is `primitives/fieldChrome.ts`, which `Input` and the new
`Textarea` both read. Copying `Input`'s style block would have fixed the
pixels and left two chromes to drift. A **global** `input, textarea`
rule is the obvious alternative and the wrong one: `Input` paints a
WRAPPER and clears the inner element inline, so a global border would sit
inside the first.

Getting the check itself right took three passes, and each wrong version
looked fine:

- Written as a Vitest assertion first, reading CSS through `?raw` —
  which Vite stubs to an empty string under Vitest. 21 files, 20 total
  characters, every class reported undefined, and it could never have
  passed. Hence the refusal below 100 defined / 100 used.
- The class half filtered to `remote-*`, so renaming a class to
  `pane-list` walked straight around it. Prefix-free now.
- The field half scanned to the first `>`, which in JSX is the arrow in
  `onChange={(e) => …}` — so it never reached `style=` and reported **70**
  false positives. It is brace- and string-aware now, and
  `checkbox`/`radio`/`file` are exempt because their chrome IS the UA's.

After all three, the repo has zero of either. Verified by reverting the
`QuickPromptsPane` fix and watching the gate name exactly those two
fields. `pnpm check:classes:self-test` forces both halves to fail so the
guard is known to be able to.

**`Input` and `Textarea` draw a focus outline, never the button
ring.** `tokens.css` documents two treatments and says which is for
which: a box-shadow `--focus-ring` (3px) for "filled chrome controls",
an outline (`--bw-focus`, 2px) for "input/list/row controls" —
`.settings-input:focus-visible` and its siblings in `envvars.css` /
`projects.css` / `banners.css` already use the second. `Input` used the
first: its inner element carried `pm-focus`, which pulls in the button
ring, stacked on the wrapper's own border turning accent-coloured on
focus. Two indicators, and the box-shadow one had nowhere to go — the
wrapper sets no vertical padding, so the ring bled 2px past the pill's
top and bottom edge instead of being contained by it. It read as one
heavy, doubled box rather than a single crisp ring.

`primitives/fieldChrome.ts` is the shared fix, read by both `Input` and
the newer `Textarea`: the wrapper's border stops changing colour on
focus, and an outline appears instead, flush with no offset — exactly
`.settings-input`'s pattern, so a field styled through the primitive and
one styled directly in a shard now agree. `focus.test.tsx` locks both
halves of the split: button-shaped primitives still carry `pm-focus`,
`Input`/`Textarea` never do, and the wrapper's `outline` (not
`boxShadow`) is what changes when the inner element gains focus. Watched
firing against the reverted state — `pm-focus` back on `Input`'s inner
element failed the "neither carries pm-focus" assertion immediately.

**A switch with no text content has no accessible name, and
`pnpm check:a11y` is the only thing that says so.** A
`<button role="switch">` holding one decorative `aria-hidden` span
announces as "switch, not checked" with nothing saying what it
switches — the visible text beside it is not a label, however obvious it
looks on screen. Two shipped that way: `SettingsSection`'s `Toggle`,
behind fourteen call sites, and `UpdatesPanel`'s, whose docstring
asserted the label was *"rendered as a sibling by the caller … same a11y
semantics"*. `SettingToggleRow` — the canonical version of the same row
— had `aria-label` + `aria-describedby` right the whole time, which is
what makes this mechanical rather than a matter of taste: the correct
pattern was already in the tree.

`Toggle`'s `label` is **required**, so tsc lists every call site rather
than leaving one to be missed.

The related-but-different failure is a name that is too LONG. A `<label>`
wrapping both a control and its explanation takes its accessible name
from all of that text, so `NetworkPane`'s probe toggle announced as
"Probe latency on open Runs a HEAD request against each endpoint…" and
`RouteForm`'s keychain checkbox as its label plus a `code`-laden note.
Both use `htmlFor` for the name and `aria-describedby` for the detail
now — different relationships, and assistive tech treats them
differently. That one is **not** gated: what counts as description is a
judgement call, and judgement calls make bad gates.

The gate's own history is the reason it is written narrowly. The first
version tried to accept a content-derived name, and the `<span>`'s
inline style object satisfied its "contains an expression" test — so
deleting `aria-label` from the real `SettingsSection` toggle still
reported OK. Watched, on the actual file. It now requires an aria
attribute outright, and both real regressions have been watched failing.

**The wide pass declares a width, and that is the whole trick.** jsdom
has no layout, so a real `ResizeObserver` measurement is always zero and
the stub used to be a no-op — which pinned every pass to the phone step
and made the wide layout unreachable by any check. It is asserted
through behaviour rather than markup (`<nav>` is true of the phone
layout too): at ≥900px, opening a thread leaves the list on screen and
there is therefore no Back chevron. Watched failing against the
pre-change shell, which reported `data-bp: sm`.

`pnpm check:render:self-test` forces a failure so the assertions are
known to fire. Note the harness **defers restoring globals to process
exit**: the panel polls on a `setInterval`, jsdom's timers are Node
timers that outlive `window.close()`, and restoring between passes let
one fire into a world with no `document` — killing the process *after*
a passing verdict had been printed.

CI runs the core + cli tests on a Linux/macOS/Windows matrix and the
`claudepot-tauri` crate's tests on macOS + Windows (Linux needs
webkit2gtk; release.yml's Linux build job is that crate's Linux
compile gate). The lint job fmt/clippy-gates `xtask` itself and runs
`cargo xtask verify-cc-parity`. Release builds preflight a five-site
version lock-step check (tag vs `Cargo.toml`, `package.json`,
`tauri.conf.json`, README status banner, web install-page banner).
