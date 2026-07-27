---
description: Bump Claudepot version numbers in lock-step across Cargo.toml, Cargo.lock, package.json, src-tauri/tauri.conf.json, README.md, the web install page, and stub a CHANGELOG section. Runs a quality gate first. Does not commit — leaves changes for review, and documents the tag/push path that follows.
---

# Bump version

Bump Claudepot's version in every file that holds one. Version lives
in five sources of truth:

| File | Line | Notes |
|---|---|---|
| `Cargo.toml` | top-level `[workspace.package] version = "X.Y.Z"` | Feeds every Rust crate via `version.workspace = true` |
| `package.json` | `"version": "X.Y.Z"` | Frontend build stamp |
| `src-tauri/tauri.conf.json` | `"version": "X.Y.Z"` | Shown to the OS (menu bar "About", bundle metadata) |
| `README.md` | `> **Status: {stage}** (\`X.Y.Z\`).` line | Public-facing status banner — the first thing visitors see in the repo |
| `web/src/app/(reader)/app/install/page.mdx` | `> **Status: {stage} (\`X.Y.Z\`).**` line | Public-facing status banner on claudepot.com/app/install — same shape, different file (mind the dot placement) |

All five MUST match byte-for-byte. A mismatch produces a release with
a wrong "About" dialog, bundles that refuse to install over previous
versions, a README that lies about the current stage, or a website
that quotes a version 10 releases old.

Two more files ride along in the same commit:

| File | Why |
|---|---|
| `CHANGELOG.md` | Stubbed in Step 4 |
| `Cargo.lock` | **Derived, but committed.** It stamps each workspace crate's own version, so a bump changes it (8 lines at v0.3.2 — four crates × two fields). Nothing edits it by hand; `cargo check` regenerates it. It is still part of the release commit — verify with `git show --stat v0.3.2`. |

**Seven files total.** Leaving `Cargo.lock` out is the easy miss: the
bump appears complete, then the next `cargo` command rewrites it and
leaves `main` dirty for whoever pulls next.

## Inputs

`$ARGUMENTS` must be one of:

- `patch` — bump the last segment (`0.0.2` → `0.0.3`)
- `minor` — bump the middle segment, zero the last (`0.0.2` → `0.1.0`)
- `major` — bump the first segment, zero the rest (`0.1.0` → `1.0.0`)
- An explicit `X.Y.Z` string — used verbatim
- `beta` — start or advance a prerelease cycle (see "Beta path" below)
- An explicit `X.Y.Z-beta.N` string — used verbatim (the beta path)

Reject anything else.

Claudepot's *numeric tier* is the release stage (see `CHANGELOG.md`
header):

- `0.0.x` — alpha
- `0.1.x` — beta tier
- `1.0.0+` — stable

The `-beta.N` *suffix* is a separate, orthogonal axis: it marks a
release on the **beta release channel** (the in-app updater's
prerelease channel — see `dev-docs/octoally-borrowings.md` Item C).
A `vX.Y.Z-beta.N` tag publishes as a GitHub prerelease and updates
only users who picked the Beta channel in Settings → About.

### Permitted vs rejected suffixes

- **Permitted:** `-beta.N` where `N` is a positive integer
  (`-beta.1`, `-beta.2`, …). This is the *only* suffix the release
  pipeline (`release.yml`) and the updater channel feature support.
- **Rejected:** every other pre-release suffix — `-alpha`, `-rc.1`,
  `-beta` with no `.N`, `-beta.0`, `-pre`, build-metadata `+…`.
  Nothing downstream handles those; accepting one would tag a
  release the pipeline can't classify.

## Procedure

### Step 0 — Quality gate, BEFORE touching any version file

```bash
cargo check --workspace
pnpm test
```

Abort the bump on any failure. This runs first, not last, for a
specific reason: a bump that stops halfway on a broken build leaves
seven files in a partially-rewritten state that the next command
happily builds on. Worse, the version bump is the last step before a
tag, and a tag naming a broken commit is public the moment it is
pushed — `release.yml` then fails on it, and the tag stays until
someone deletes it by hand.

Step 5 re-runs the gate after the edits. Both passes are wanted: this
one proves the tree was sound before, that one proves the edits didn't
break it.

### Step 1 — Read the current version

Read `Cargo.toml` line `version = "..."` under `[workspace.package]`.
That's the authoritative current version. Parse as `CURRENT = X.Y.Z`.

Confirm all five locations agree (the three manifests plus both
status banners). If they don't, STOP and report the drift — bumping
from a drifted state would silently adopt one file's opinion as
canonical.

### Step 2 — Compute the next version

Apply the rule from `$ARGUMENTS`:

| Input | Rule |
|---|---|
| `patch` | `X.Y.(Z+1)` |
| `minor` | `X.(Y+1).0` |
| `major` | `(X+1).0.0` |
| `X.Y.Z` literal | use as-is, validate it's strictly greater than `CURRENT` |
| `beta` | see "Beta path" below |
| `X.Y.Z-beta.N` literal | use as-is, validate per "Beta path" below |

Store as `NEXT`.

#### Beta path

The `-beta.N` suffix advances a prerelease *toward* an unreleased
target `X.Y.Z`. Two cases:

1. **`CURRENT` already carries a `-beta.N` suffix**
   (e.g. `0.2.0-beta.1`): the `beta` argument bumps the suffix —
   `X.Y.Z-beta.(N+1)`. The numeric `X.Y.Z` core is unchanged. So
   `0.2.0-beta.1` + `beta` → `0.2.0-beta.2`.
2. **`CURRENT` is a plain `X.Y.Z`** (no suffix): a bare `beta`
   argument is ambiguous — it doesn't say which `X.Y.Z` the beta is
   *for*. STOP and ask the user to pass an explicit
   `X.Y.Z-beta.1` literal naming the target version (e.g.
   `0.2.0-beta.1` to start the beta cycle for the eventual `0.2.0`).

For an **explicit `X.Y.Z-beta.N` literal**:

- Validate the shape: `X`, `Y`, `Z`, `N` all non-negative integers,
  `N ≥ 1`, and the suffix is exactly `-beta.N` (reject `-beta`,
  `-beta.0`, `-rc.*`, `-alpha`, `+build`).
- Validate ordering: `NEXT` must be strictly greater than `CURRENT`
  under SemVer precedence — a `-beta.N` prerelease sorts *before*
  its release `X.Y.Z` and after `X.Y.Z-beta.(N-1)`. So
  `0.2.0-beta.2 > 0.2.0-beta.1`, and `0.2.0 > 0.2.0-beta.9`.
- **Reject a beta for an already-released version.** Because a
  `-beta.N` sorts *before* its `X.Y.Z`, a beta whose core is
  ≤ `CURRENT`'s released version is a backwards bump. Examples:
  - `CURRENT 0.2.0`, literal `0.2.0-beta.1` → **reject** (the beta
    precedes its own already-shipped release).
  - `CURRENT 0.2.0`, literal `0.2.1-beta.1` → accept (beta for the
    *next* version).
  - `CURRENT 0.1.39`, literal `0.2.0-beta.1` → accept.
- The eventual stable release of that cycle is bumped later with a
  normal `minor`/`patch`/literal argument to the plain `X.Y.Z` — the
  `-beta.N` tags are stepping stones, not the destination.

### Step 3 — Apply edits

When `NEXT` is a `X.Y.Z-beta.N` version, write the **full string
including the suffix** into every one of the five locations below
(Cargo.toml, package.json, tauri.conf.json, and both status
banners). The version strings must still match byte-for-byte. The
**stage word** is derived from the numeric `X.Y.Z` core only — the
`-beta.N` suffix does not change the tier (`0.2.0-beta.1` → tier
`0.2.x` → stage `beta`). The suffix in the public status banners is
intentional: it tells visitors the build is a prerelease.

Edit exactly these five locations:

1. `Cargo.toml` → the `version = "CURRENT"` line under
   `[workspace.package]`. Use `Edit` with the full surrounding line to
   avoid accidentally rewriting a crate's dev-dep version pin.
2. `package.json` → the top-level `"version": "CURRENT"` field. Keep
   the surrounding JSON formatting (2-space indent, trailing comma
   where present).
3. `src-tauri/tauri.conf.json` → the top-level `"version"` field.
4. `README.md` → the status banner near the top (find the line
   matching `> **Status: <stage>** (\`X.Y.Z\`).`). Rewrite both
   the stage word and the version, since a major-tier crossing
   (e.g. `0.0.x` → `0.1.x`) flips `alpha` → `beta`. Stage rule is
   the same as Step 4's CHANGELOG rule:
   - `0.0.x` → `alpha`
   - `0.1.x` → `beta`
   - `1.0.x`+ → `stable`
   The rest of the line ("Daily-driven on macOS…") stays untouched —
   that's editorial copy, not version-derived.
5. `web/src/app/(reader)/app/install/page.mdx` → the status banner near
   the top. Same rewrite as README — stage word + version, no other
   prose. Note the bold scope differs from README's: README is
   `> **Status: beta** (\`X.Y.Z\`).` (bold around "Status: beta"
   only); the MDX is `> **Status: beta (\`X.Y.Z\`).**` (bold extends
   across the version and trailing period). Edit by matching the
   `X.Y.Z` token plus its backticks, not the surrounding markdown.

Then sync the lockfile. It is derived, so never hand-edit it — but it
**must** land in the same commit:

```bash
cargo check --workspace          # rewrites Cargo.lock in place
git diff --stat Cargo.lock       # expect ~8 changed lines (4 crates)
```

If `git diff --stat Cargo.lock` shows nothing, the bump did not take —
stop and re-check Step 3's `Cargo.toml` edit. If it shows far more than
the workspace crates, a dependency moved too; that is a separate change
and does not belong in a bump commit.

### Step 4 — Stub CHANGELOG

Prepend a new section to `CHANGELOG.md` immediately after the
three-line `Versioning scheme:` block:

```markdown
## NEXT — {stage} (unreleased)

### Added

- _…list user-visible additions…_

### Changed

- _…list user-visible changes…_

### Fixed

- _…list user-visible fixes…_
```

Where `{stage}` is derived from `NEXT`:

- `0.0.x` → `alpha`
- `0.1.x` → `beta`
- `1.0.x`+ → `stable`

Leave the bullet placeholders — the user fills them in as part of the
release process. If an unreleased section already exists for `NEXT`,
do NOT duplicate it; report "CHANGELOG already has a section for NEXT"
and stop.

### Step 5 — Verify

Run these checks in parallel and report any failure:

```bash
cargo check --workspace
pnpm build
```

Show the final diff:

```bash
git diff --stat            # expect 7 files, Cargo.lock among them
git diff Cargo.toml package.json src-tauri/tauri.conf.json CHANGELOG.md \
         README.md 'web/src/app/(reader)/app/install/page.mdx'
```

If `git diff --stat` lists six files, `Cargo.lock` did not regenerate —
go back to Step 3's lockfile sync. That is the miss this command exists
to prevent.

### Step 6 — Do NOT commit

Leave the changes staged-but-uncommitted. Version bumps ride with a
release commit that usually also adjusts `CHANGELOG.md` prose and
maybe `dev-docs/`. The user drives that final edit pass.

End with a one-line summary:

```
Bumped CURRENT → NEXT. 7 files changed (incl. Cargo.lock). Review + commit when ready.
```

## After the bump — the release path

**This command does not run any of the following.** It stops at Step 6.
The steps are documented here because the bump is the only surface that
leads into them and the hazards below were previously written down
nowhere in this repo.

`main` is **not** branch-protected here, so the bump commit goes
straight to `main` — no PR dance. (If that ever changes, the tag must
name a commit already merged, since required checks cannot have run on
a commit the remote has never seen.)

```bash
git add Cargo.toml Cargo.lock package.json src-tauri/tauri.conf.json \
        CHANGELOG.md README.md 'web/src/app/(reader)/app/install/page.mdx'
git commit -m "Release {NEXT}: {one-line theme}"
git push origin main            # let CI go green BEFORE tagging
git tag v{NEXT}
git push origin v{NEXT}         # the SINGLE new tag, never --tags
```

### Never `git push --tags`

`release.yml`'s cleanup step runs
`gh release delete --cleanup-tag` (line ~957), which **deletes the tag
from origin** as it prunes old releases past the keep-count. Those tags
survive in every local clone. `git push --tags` re-pushes the whole
stale set, and each resurrected `v*` tag re-triggers a full release
run — matrix builds, signing, the lot.

Push the one new tag by name. Always.

### A tag push runs the validators, and holds SSH open

Per AGENTS.md, the `pre-push` hook fires the Linux + Windows validators
**only** when the push contains a `refs/tags/v*` ref. That is two SSH
round-trips running clippy and a Windows compile — minutes — while git
keeps the connection open.

If the push dies with **exit 141 (SIGPIPE)** after the gate reports
green, that is the SSH connection timing out, not a quality failure.
Retry with:

```bash
GIT_SSH_COMMAND='ssh -o ServerAliveInterval=20' git push origin v{NEXT}
```

`--no-verify` is **not** the fix. AGENTS.md records v0.2.10 through
v0.2.12 all shipping that way while the validators sat inert, and notes
that a bypass used routinely is indistinguishable from no gate at all.
Push the branch first, let CI finish, then push the tag — that is the
workflow that keeps the gate real.

## Rules

- One argument only. Reject `$ARGUMENTS` that combines a keyword
  (`patch` / `minor` / `major` / `beta`) with a literal version.
- The only accepted pre-release suffix is `-beta.N` (`N ≥ 1`), via
  the `beta` keyword or an explicit `X.Y.Z-beta.N` literal. Reject
  every other suffix — `-alpha`, `-rc.*`, bare `-beta`, `-beta.0`,
  `+build` metadata. See "Permitted vs rejected suffixes" above.
- Refuse to bump backwards (`NEXT <= CURRENT`) under SemVer
  precedence — this includes prerelease ordering, so a `-beta.N`
  must exceed `CURRENT` (whether `CURRENT` is a plain release or an
  earlier `-beta`). If the user really wants a backwards bump, they
  can edit the files directly.
- Refuse if the working tree is dirty in any of the seven touched
  files (Cargo.toml, Cargo.lock, package.json,
  src-tauri/tauri.conf.json, CHANGELOG.md, README.md,
  web/src/app/(reader)/app/install/page.mdx) — let the user commit
  or stash first so the bump diff is isolated. `Cargo.lock` counts:
  a pre-existing lock change would ride along invisibly inside what
  looks like a pure version bump.
- Do not touch any other file, and do not touch other parts of the
  seven files. Specifically: in README.md only the status banner
  line is in scope; do not retouch install snippets, version
  strings in code blocks, or anything else. Version strings in doc
  examples (e.g. `dev-docs/*.md`) are intentionally pinned and must
  not drift with the bump.
- Do not run `cargo build --release` or `pnpm tauri build` — those
  are release-step work, not bump-step work.
