# Homebrew publishing

Claudepot publishes to the `xiaolai/homebrew-tap` tap. Users install with:

```bash
brew tap xiaolai/tap

# macOS — one command, gives you both GUI and CLI:
brew install --cask claudepot

# Linux — CLI only (no GUI target on Linux):
brew install claudepot
```

On macOS the CLI is shipped **inside** `Claudepot.app` (via Tauri's
`externalBin` feature) and the cask's `binary` stanza symlinks it as
`/opt/homebrew/bin/claudepot`. One install, both surfaces, one signed
and notarized bundle. On Linux the cask is not available at all (no
Linux GUI target in the release matrix), so the formula is the only
path — and by design it is `depends_on :linux` so `brew install
claudepot` on macOS errors out early with a clear message telling
users to use `--cask` instead.

## Files in this directory

- `Formula/claudepot.rb` — Linux CLI formula **template** (committed
  reference copy). Two per-arch Linux tarball URLs,
  `bin.install "claudepot"`, test invokes `--help` and asserts the
  tagline.
- `Casks/claudepot.rb` — GUI cask **template** (committed reference
  copy). Two per-arch DMG URLs, `binary` stanzas that expose the
  bundled CLI as `claudepot`, `livecheck` via `:github_latest`, `zap
  trash:` list covering `~/.claudepot/`, WebKit, caches, prefs, saved
  state.

These are *templates with `REPLACE_ME` placeholders for sha256s*. They
document the expected shape and are useful for:

1. **Manual first publish** — when `v0.0.2` ships and the automated
   workflow has never run, an operator can hand-compute sha256s and
   open a PR against the tap using these as the starting point.
2. **Reading** — reviewers can see the shape of the formula/cask
   without chasing heredocs inside the workflow.

The **canonical source** is the heredoc inside
`.github/workflows/update-homebrew.yml`. That workflow rewrites both
files from scratch on every release. The committed templates here are
allowed to drift in inessential ways but must stay in shape-parity
with the heredoc.

## How the CLI ends up inside Claudepot.app

The macOS release job in `.github/workflows/release.yml`:

1. Builds `claudepot` (CLI) with `cargo build -p claudepot-cli
   --release --target <triple>`.
2. Signs and notarizes it as a standalone zip (for users who just
   want the binary without the GUI — still published as a release
   asset, just not consumed by Homebrew anymore).
3. **Stages** a copy at `src-tauri/binaries/claudepot-cli-<triple>`
   so Tauri's `externalBin` config picks it up.
4. Runs `pnpm tauri build --bundles app,dmg`. Tauri places the CLI
   at `Claudepot.app/Contents/MacOS/claudepot-cli-<triple>` and
   deep-signs the whole bundle as part of its normal signing +
   notarization step.

The `src-tauri/binaries/` directory is gitignored — it only exists
during CI.

## How the auto-publish works

`.github/workflows/release.yml` ends with two steps:

1. **Publish release** — flips the draft release to published via
   `gh release edit --draft=false`. Auto-publish is the default because
   `update-homebrew.yml` can't fetch draft release assets (they require
   auth). Comment out this step if you want manual gate on each
   release.
2. **Trigger Homebrew tap update** — fires
   `gh workflow run update-homebrew.yml -f version="<version>"`.
   `continue-on-error: true` so a tap failure doesn't mark the release
   as failed.

`update-homebrew.yml` then:

1. Polls four release asset URLs (two DMGs + two Linux tarballs)
   until all return 2xx/3xx (15-min ceiling). macOS CLI zips are
   *not* consumed — the CLI now rides inside the .app.
2. Downloads each, computes sha256, stores in step outputs.
3. Checks out `xiaolai/homebrew-tap` with `HOMEBREW_TAP_TOKEN` (PAT
   with `contents: write` on that repo).
4. Regenerates `Formula/claudepot.rb` and `Casks/claudepot.rb` from
   heredocs with the computed shas.
5. Commits and pushes with message `Update claudepot to <version>`.

## First-release manual publish (until automation is proven)

1. Tag and push: `git tag -a v0.0.2 -m "v0.0.2" && git push origin v0.0.2`.
2. Wait for `release.yml` to finish, inspect the draft release, publish it.
3. Download the four brew-relevant assets, compute sha256:
   ```bash
   for f in claudepot-aarch64-linux.tar.gz claudepot-x86_64-linux.tar.gz \
            Claudepot-aarch64.dmg Claudepot-x86_64.dmg; do
     curl -sL -o "/tmp/$f" \
       "https://github.com/xiaolai/claudepot-app/releases/download/v0.0.2/$f"
     echo "$f: $(sha256sum "/tmp/$f" | cut -d' ' -f1)"
   done
   ```
4. Copy `Formula/claudepot.rb` and `Casks/claudepot.rb` into your
   local checkout of `xiaolai/homebrew-tap`, substitute the sha256s
   and the version string.
5. Verify locally:
   ```bash
   brew install --cask ./Casks/claudepot.rb     # should drop GUI + CLI
   which claudepot                              # /opt/homebrew/bin/claudepot
   claudepot --help                             # tagline check
   ```
   On a Linux box:
   ```bash
   brew install --build-from-source ./Formula/claudepot.rb
   ```
6. Commit `Update claudepot to 0.0.2`, push, and that's the live tap.

Once the manual round works, wire up the `HOMEBREW_TAP_TOKEN` secret
on this repo and let the next release's automation take over.

## Secrets required

| Name | Scope | Where |
| --- | --- | --- |
| `HOMEBREW_TAP_TOKEN` | PAT with `contents: write` on `xiaolai/homebrew-tap` | this repo's Actions secrets |

## Out of scope for the cask zap

- `Claude Code-credentials` Keychain item — shared with Claude Code
  itself; removing it would break the user's active CC install.
- `com.claudepot.keys.api`, `com.claudepot.keys.oauth` Keychain items
  — Claudepot-owned OAuth blobs, but casks cannot run `security`
  commands. Users who want a full wipe remove these manually via
  Keychain Access.
