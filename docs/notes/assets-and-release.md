# Icons, screenshots and release validation

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries three pipelines whose failures all look like something else.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## Icon assets

Full post-mortem of the v0.1.13–0.1.19 Dock-blur arc is in
`dev-docs/icon-design-notes.md`.

**The authored set lives in `assets/icon-set/`** — isometric block on
an anodised plate, every coordinate a multiple of 16 on a 1024 grid.
That directory is the source; `src-tauri/icons/` holds only what
`scripts/regen-icons.sh` derives from it, plus the two masters the
script reads directly (`icon.svg`, `icon-flat.svg`). There is
deliberately no second copy of the artwork anywhere: the previous
`pixel-*` masters were deleted when this landed rather than left
beside it, because two plausible masters in one directory is how the
wrong one gets regenerated from.

Load-bearing rules:

- **SVG must use a power-of-2-friendly grid.** The current set is on
  16-unit multiples in a 1024 viewBox. Avoid 22, 28, 30 — they don't
  divide 128/256 cleanly and rsvg AA-softens at every Dock size.
- **Generate raster icons via `scripts/regen-icons.sh`,
  not `pnpm tauri icon`.** The latter uses lossy resampling for
  some `.icns` layers and produces ~50 dead-byte files for targets
  we don't ship (iOS, Android, MSIX). Our script uses
  `rsvg-convert` + `iconutil` + a manual ICO struct-pack that
  embeds PNG-compressed layers verbatim.
- **Three masters, not one, and the split is not cosmetic:**
  - `icon.svg` — plated master with an `feTurbulence` grain, used at
    48 px and up.
  - `icon-flat.svg` — same artwork, solid plate, no filter. Used
    **below 48 px**. The grain is computed at render size, so it
    coarsens relative to the tile as the tile shrinks and reads as
    dirt rather than as a finish. A single-source ladder cannot
    express this.
  - `assets/icon-set/windows/icon-glyph.svg` — plateless, for
    `icon.ico`. Windows draws no enclosure and shows the icon against
    chrome of every shade, so the plate would read as a grey card
    floating behind the block.
- **Tray icons are generated too** (`tray-icon{,Alert}{Template,Mono}@2x.png`,
  44×44). Template is inverted by macOS to match the menubar; Mono is
  the same alpha filled `#808080` because Windows and Linux have no
  template concept and a pure-black glyph vanishes on a dark taskbar.
  - **`tray-icon` normalises the tile to 18 points tall, so the tile's
    pixel size is irrelevant and its padding is pure loss.** The crate
    hard-codes `let icon_height: f64 = 18.0` and derives width from the
    aspect ratio, so only the FRACTION of the tile the glyph inks decides
    how large it lands in the menubar. Measured against its neighbours:
    ChatGPT.app inks 94.4% and renders 17.0pt; Claude.app inks 70.8% and
    renders 12.8pt. Claudepot inked 77% and rendered 13.9pt — visibly
    smaller, while passing every dimension check, because those checks
    asserted a tile fraction rather than the thing the user sees. The
    tray SVGs carry a cropped `viewBox` (672 of the 1024 authoring
    canvas) and now render 16.4pt. Both variants share one viewBox
    *size* so the block does not change scale when the alert badge
    appears, and **the badge sets the floor on how tight the crop can
    go** — it moved inward to (692, 332) to buy it. Its margin is derived
    in RENDERED PIXELS and converted back: a unit here is 44/672 px, so a
    first attempt at a 12-unit margin measured 0.79px, antialiasing
    closed it, and the badge rendered touching two tile edges.
    `verify-icons.py` asserts the rendered POINT HEIGHT and that nothing
    touches the tile edge — every dimension check passed while the icon
    was too small, so the rendered-height assertion is the one that
    matters.
- **`scripts/verify-icons.py` is the structural gate** — 58 checks over
  the PNG ladder, the `.icns` layer list, ICO layer encoding, tray
  sizes, and the grain floor. It catches the failures that still look
  like valid files on disk: an ICO whose layers are raw BMP, an `.icns`
  missing the 128/256 layers the Dock reaches for, a small raster that
  kept the grain. Run it after any icon change. It needs no GUI; what
  it explicitly does **not** check is how the artwork looks, which is
  what launching the app is for.
- **The bundle path and the `setIcon` path want OPPOSITE artwork, and
  swapping them is the classic macOS icon bug:**

  | Path | Wants |
  |---|---|
  | bundle `.icns` / `bundle.icon` list | **full bleed** — macOS applies the squircle mask, inset and shadow |
  | `setApplicationIconImage` (`dock_icon.rs`) | **everything already applied** — drawn verbatim at slot size, no mask, no inset, no shadow |

  So `dock_icon.rs` embeds `icon-dock.png`, **not** `icon.png`: 1024
  canvas, artwork inset to 824/1024 = 0.805 (Apple's measured tile
  fraction), superellipse corner (`|x/a|^n + |y/a|^n = 1`, n = 5) rather
  than a circular arc, which meets the straight edge with a curvature
  discontinuity and reads boxy beside real icons. A full-bleed image on
  this path renders as a hard square measured **~22% larger** than every
  neighbouring Dock icon.

  The pre-2026-08 artwork hid the distinction by baking a squircle into
  the SVG at 416/512 = 0.813 of the canvas, so one file happened to
  serve both roles. The current set is full-bleed by design — correct
  for the bundle — which is exactly why the second asset now exists.
  Reference: `~/.claude/agents/icon-smith/specs.md`, measured against
  macOS 26.5.
- **`src-tauri/src/dock_icon.rs` calls `setApplicationIconImage`
  at startup on macOS.** This is required — Tauri's runtime only does
  this in dev mode. Without it, prod Dock at default size (96 px on
  Retina) renders the `.icns` 128 layer downscaled bilinearly and looks
  visibly soft. The 1024-px source means every Dock size is a clean
  Lanczos downsample.
- **`pnpm tauri icon`'s output paths are `.gitignore`'d** so a
  stray invocation can't re-stage MSIX/iOS/Android dead bytes.

## Documentation screenshots

No manual capture, no PII scrubbing:

```bash
cargo xtask screenshot-fixture                  # synthetic profile
pnpm dev &                                      # vite, REAL home
cargo build -p claudepot-tauri                  # REAL home
HOME=/tmp/claudepot-demo-home \
  ./target/debug/claudepot-tauri &              # app only
pnpm screenshots                                # capture all 9
```

**Quit the installed Claudepot first.** The debug binary carries the
same `tauri-plugin-single-instance` identifier as the release app, so
launched beside a running `/Applications/Claudepot.app` it hands off to
that instance and exits 0 with nothing in its log but the startup line
— and `pnpm screenshots` then reports "no MCP bridge on 9223", which
reads as a bridge fault. `osascript -e 'tell application "Claudepot" to
quit'`, capture, then `open -a Claudepot`.

The fixture is a **fake `HOME`**, not a pair of env overrides.
`CLAUDE_CONFIG_DIR` + `CLAUDEPOT_DATA_DIR` cover only two of the three
places the app reads — Claude Desktop's directory resolves through
`dirs::data_dir()` with no override and leaked a real account through
the header. `HOME` closes every home-relative path at once.

Two things that look like fussiness and are not:

- **The fake home goes to the app, not the build.** `HOME=… pnpm tauri
  dev` reads better and fails — rustup keeps its default toolchain in
  `$HOME/.rustup`, so the override takes the toolchain with it and
  `cargo metadata` dies before anything compiles.
- **The fixture lives outside the repo** (`/tmp/claudepot-demo-home`).
  The app displays the paths it reads, so an in-repo fixture put
  `/Users/<you>/…/claudepot-app/fixtures/…` on screen in Global →
  Config. No amount of synthetic *data* fixes a leaking *path*.

**Never mask real data to take a screenshot.** It was tried and it is
architecturally wrong: the vocabulary is unbounded and only visible as
you navigate (1 project name found on one surface, 79 across four), and
substring replacement corrupts legitimate UI — a harvested `claude`
turned `.claude/settings.json` into `vector-store/settings.json` and the
`CLAUDE-F…` model badge into `SEARCH-INDEX-F…`. Free text defeats it
entirely. Full reasoning in `crates/xtask/src/screenshot_fixture.rs`.

`scripts/capture-screenshots.mjs` drives the app over the MCP bridge's
WebSocket (plain JSON, no auth) and writes both `assets/screenshots/`
and `web/public/screenshots/`. Node, not xtask, so it needs no new
dependency. Each shot waits for a `settle` string rather than sleeping,
and a pane that never settles is **skipped, never captured blank**.

Adding a screenshot means two edits: a `SHOTS` row in the capture
script, and a `SCREENSHOTS` row in `crates/xtask/src/verify_docs.rs`.

Two checks read that table, and the split is deliberate:

- **`cargo xtask verify-docs`** (runs in CI) asserts each shot exists and
  that `assets/screenshots/` and `web/public/screenshots/` hold the same
  bytes. Content-based, no false positives, and the fix is a file copy —
  something a red CI run can actually ask you for.
- **`cargo xtask verify-screenshots`** (**on demand**, not a PR gate)
  reports shots whose sources have moved since capture. Run it before a
  release, or after changing a view you know is captured.

Freshness is not a gate for two reasons. It compares **commit dates, not
mtimes** — `git checkout` rewrites mtimes, which is how eight screenshots
sat three months stale unnoticed — but it compares them per *directory*,
so any edit under `src/sections/projects` reads as "the UI changed",
including edits to views no screenshot shows. And re-capturing needs a
macOS GUI session, a Vite server, a debug build carrying the MCP bridge
and a windowed app; CI has none of them, so a failure there is a wall
rather than a signal. A gate whose remedy cannot run where it fires is
the dynamic that made `--no-verify` a reflex for the release validators.

Adjacency is not staleness. When `verify-screenshots` flags a shot whose
captured view provably did not move, that is the check being coarse —
say so, rather than re-capturing to silence it.

Known limitation: `HOME` does not redirect the macOS keychain, so the
Accounts pane's live credential probe finds nothing and each card shows
"Saved login is missing or broken".

## Release validation (Linux + Windows)

CI's clippy + Windows-test gates run on Linux/Windows runners that
local macOS can't reproduce. A four-round cascade of "fix-and-pray"
clippy commits in v0.0.18 prompted this setup:

- **`<runner-a>`** (internal validator network, Ubuntu aarch64) —
  runs the same command as CI's `Format / Clippy (Linux)` job:
  ```bash
  cargo clippy --all-targets -p claudepot-core -p claudepot-cli -- -D warnings
  ```
  Catches new-clippy-version lints (1.95 added `io_other_error`,
  `manual_pattern_char_comparison`; 1.92 added `useless_format`,
  `cloned_ref_to_slice_refs`, `iter_nth_zero`) and
  `cfg(target_os = "macos")`-only items that the macOS-local clippy
  never sees. `--all-targets` covers test-code lints too — without
  it, test-only drift accumulated silently between 1.92 and 1.95
  and surfaced as a 7-lint backlog on 2026-05-13.

- **`<runner-b>`** (internal validator network, Win 11 MSVC x86_64) —
  runs the same compile-step as CI's `Tests (windows-latest)` job:
  ```bash
  cargo test -p claudepot-core -p claudepot-cli --no-run
  ```
  Catches Windows-only compile errors (e.g. types referenced in
  `cfg(target_os = "windows")` arms but cfg-gated to macOS only).

Real host names and the network they sit on live in `CLAUDE.local.md`
(gitignored).

The hook source is committed at `scripts/pre-push`. Install it
per clone with `scripts/install-hooks.sh`. The hook auto-runs both
validators against the pushed SHA when — and only when — the push
contains a `refs/tags/v*` release tag. Branch pushes skip
validation. Failure aborts the push and prints the recovery recipe
(delete tag, fix locally, re-tag, re-push).

**Never hand-symlink the hook into `.git/hooks/`.** A global
`core.hooksPath` — set by the git-lfs installer and most dotfile
setups — makes git ignore `.git/hooks` entirely, so a symlinked hook
reports "Installed" and then never runs. The v0.2.7 … v0.2.10 tags
were all pushed with the validators silently inert for exactly this
reason. `install-hooks.sh` instead points `core.hooksPath` at a
generated, gitignored `.githooks/` (a `--local` setting, so no other
repo is affected) whose hooks call `scripts/<hook>` and then chain to
whatever the clone previously inherited — the global `commit-msg`
and git-lfs hooks keep working. Re-running is safe; the inherited
path is recorded once in `claudepot.inheritedHooksPath`.

Verify an install rather than trusting it: `git config
core.hooksPath` should print a `.githooks` path, and a dry-run push
of a throwaway `v*` tag should print the validator banner.

**When a validator host is unreachable, the hook defers to CI** rather
than failing. CI runs the same two gates on the same commit, so the
hook asks `gh` whether the `ci.yml` run for that commit is green and
accepts it in place of the missing host. Note the asymmetry: a host
that is *reachable and fails* still aborts. Only absence of evidence
falls back, never contrary evidence.

"Runs the gate" is the operative phrase, and it has been misjudged
twice in the same direction: a host that could not reach GitHub (v0.3.1)
and a host that could not download a crate from crates.io (v0.6.2, a
30 s timeout on `wayland-scanner`) were both reported as "clippy
failed" over a gate that never started. The hook now syncs the commit
and runs `cargo fetch --locked` as separate steps, and a failure in
either is absence of evidence. Only the gate command's own exit is
contrary evidence.

The lookup dereferences `^{commit}` first — an annotated tag's own
object sha differs from the commit's, and CI indexes runs by commit,
so looking up the tag sha would silently never match. It also needs
an authenticated `gh`; an unauthenticated one reads as "no run" and
aborts, which is the safe direction.

This exists because `--no-verify` was becoming the reflex — v0.2.10,
v0.2.11 and v0.2.12 all shipped that way while the validator boxes
were offline. A bypass used routinely is indistinguishable from no
gate at all, which is how these validators sat inert for four
releases. The workflow that keeps the gate real: push the branch,
let CI finish, then push the tag.

Validator hosts are never committed: the hook reads them from the
gitignored `.validator-hosts` file at the repo root (shape documented
in the `scripts/pre-push` header) or from
`CLAUDEPOT_VALIDATOR_LINUX_SSH` / `CLAUDEPOT_VALIDATOR_WINDOWS_SSH`
in the environment. Real host names live in `CLAUDE.local.md`.
Bypass with `git push --no-verify` if a host is unreachable, but
note CI is unforgiving about red main.
