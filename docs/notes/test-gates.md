# The test gates — what each one exists to catch

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries the reasoning. Every gate here was written after a
specific failure shipped green, and this is the record of which one.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## What each gate answers

The commands are in `AGENTS.md`'s `## Test`; this is what each one
answers that nothing else does.

| Gate | The question it answers that nothing else does |
|---|---|
| `check:render` | does the **committed** panel bundle *mount*? Seven passes: signed-out and offline, signed-in thread, the quick-prompt sheet, the slash-command sheet, staging a command, the offline queue as a round trip, the 1200px two-pane layout. `vite build` cannot answer it — it does not resolve free identifiers, so a missing import is a runtime `ReferenceError` in whatever path touches it |
| `check:layout` | does the **committed** panel bundle lay out, in real Chrome, at 280, 320, 390, 480, 700, 900, 1200 and 1700? `check:render` has no layout and `panel/src/app/widths.test.js` reads source, so neither sees geometry: of 26 sideways regressions, those source scans caught 3. Across Sessions, Accounts, Settings and two threads (one with an empty project path), plus a pass with over a hundred live sessions, one where the session list answers slower than the poll and approvals never answer, and one where a thread's first read fails and must recover unaided, it measures a list row's details staying on one line with its warning and its project both kept, however long the name, and a shortened name leaving none of its line empty; text cut short with no `title`; meter spacing and labels; the thread's Copy path target; the widest badge against the rail's edge; the composer growing, keeping the last message in view, capping, wrapping and shrinking back as real input is typed; a separator opening a wrapped line, hidden or transparent; details, badge, tab icons or composer at exactly zero opacity, `visibility` hidden or a colour or stroke of exactly zero alpha, and tab icons with no size; a badge over its icon or away from it; the active tab icon being non-empty and in its accent colour; the composer holding one line and staying on screen; the list scrolling to its last row; the list column not falling below 30% in the two-pane layout; and an element past its nearest clipping ancestor, sideways for any box (empty absolute boxes and shapes inside an `svg` exempt) and vertically for leaf text, which adds no scroll for a page-overflow check to see. The tabs, the two rows the walk opens, the composer and the Settings choices must own the point under their centre, so an overlay, `pointer-events: none` or a `clip-path` over that point fails. A view that did not render, a `/panel/` file the walk fetched that 404s, or a font or chunk the bundle names and lacks fails. Waits run 60s because a renderer was measured stalling 30-40s on a loaded Mac (the composer steps wait on drawn frames and give up after 5s, as `no-frame`); one that runs out reports the requests made during it, the slowest page answer and the page's text, which is evidence and not a diagnosis (a broken panel can make no requests, and a blank one can make some). Known limits: desktop Chrome only (no iOS Safari or touch); a stubbed API, so the approval card, login, command picker and tool ticks are never shown (`widths.test.js` reads their source); one window height (800) and the light theme; only the widths listed; overlapping rows are not checked; this catches common ways to hide something, not every way. Near-zero alpha, a colour matching its background, `filter`, `mask`, blend modes, a `clip-path` off centre, hidden paths inside an icon and an overlay missing every centre point pass; `transform` and `font-size: 0` are caught only as a side effect of other checks, not reliably. Needs Chrome: exit 2 locally without one, 1 under `CI`; a Chrome that dies mid-run is 1 |
| `check:classes` | does every `className` have a CSS rule behind it, and does every bare `input` / `textarea` draw chrome? Both are valid HTML and invisible to `tsc`. Refuses a vacuous pass under 100 defined / 100 used; `lucide*` is exempt |
| `check:a11y` | does every `role="switch"` have an accessible name? Requires an aria attribute outright — the visible text beside a switch is not a label |
| `check:motion` / `check:contrast` | do the `prefers-reduced-motion` and `prefers-contrast: more` overrides actually *reach* the primitives, which animate from inline styles? Both turn on source order inside `tokens.css` |
| `check:container-queries` | can every `@container` rule apply? A query matches an ancestor, so a rule selecting the element that declares the container never fires, a query naming no declared container never fires either, and a queried property also set inline on its target loses to the inline value. Known limits: it reads only plain-string and template `className`s (not `cx(...)` or a conditional), and its overlap list covers `border`/`padding`/`margin` sides and `flexFlow`, not `gap`, `paddingInline`, `borderWidth`, `flexGrow` or `inlineSize`. The CSS is valid and the rest of the block still works, so the layout half-changes. Activities' filter rail did: it took its narrow shape while its parent stayed a row, and pushed the card stream out of the window |
| `check:inline-flow` | does text inside a flex or grid container keep its spaces? Each text run there becomes its own flex item and loses its edge white space, so `<Trans components>` rendered "clickReindexto" while `textContent` — all a render test sees — still had the spaces. Reads inline styles and single-class CSS rules; an unresolvable parent passes |
| `check:catalogs` | en↔zh parity. "Orphan" is **cross-locale only** — it cannot see a key that no source file references, and a green run is not evidence there are none |
| `check:envvar-layout` | does the env-var pane lay out at all, and does its list stay inside the pane at the 800px minimum window? Drives the real app over the debug-only MCP bridge, so CI runs the pure `evaluate()` half instead. The list is windowed, so the sideways check sees the rows rendered at that moment, not all ~400 |

Eight carry a `:self-test` that forces the assertions to fail
(`check:classes`, `check:a11y`, `check:motion`, `check:contrast`,
`check:container-queries`, `check:inline-flow`, and the panel's
`check:render` and `check:layout`), `check-envvar-layout.mjs` takes
`--self-test`, and `check:catalogs` is exercised by pointing
`CLAUDEPOT_LOCALES_DIR` at a fixture. A check nobody has watched fail
is indistinguishable from one that cannot fail.

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

**Text directly inside a flex or grid container loses its spaces, and
`pnpm check:inline-flow` is the only thing that says so.** Each run of
text there becomes its own anonymous flex item, and white space at the
edge of an item is discarded. `<Trans components>` returns text, element,
text as siblings, so the Activities empty state — catalog string "or
click <em>Reindex</em> to backfill" — rendered as "or clickReindexto
backfill". It had been like that in the committed screenshot since
2026-09-05 and nobody saw it. `textContent` keeps the spaces, so a
render test asserting on text passes, and `tsc` has no view of layout.

Found by eye during a screenshot re-capture on 2026-09-18, then swept
for as a class: a static pass over all 110 `<Trans>` call sites and a
DOM pass over 37 views of the running app. Four sites, all fixed by one
`<span>` wrapper, which is a single flex item inside which the text
flows as a line again — the Activities empty state, the Providers form's
"Enable <code>tool_reference</code> beta blocks" label, and two
`.envvar-note` paragraphs. Those last two are flex **only through a CSS
class**, which is why the check reads the stylesheets and not just
inline styles; a check that read only `style=` would have found half.

What it deliberately does not flag, because each was measured and would
have been noise:

- **Text beside an expression** — `Enable {name} blocks`. Contiguous
  text runs are wrapped in one anonymous item, so those spaces survive.
  The first draft flagged them anyway.
- **Spacers** — `{" "}` (56 in `src/`) and a same-line
  `<Glyph /> {label}` (38). Flex discards them too, but the rows that
  hold them set a `gap`, which is the spacing the reader actually sees.
  Three sat in resolvable flex parents; all three were invisible. A gate
  that reports invisible things teaches people to ignore it.
- **A parent it cannot resolve** — a component, a computed `className`,
  a descendant or state selector. It misses those rather than guessing.

`pnpm check:inline-flow:self-test` plants six defects (inline style,
class rule, spread const, conditional `grid`, fragment) and nine
lookalikes that must pass. The real run was watched red on all four
sites before the fix and refuses a vacuous pass under 100 files or 20
flex/grid classes.

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
