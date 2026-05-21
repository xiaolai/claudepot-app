---
description: Bump Claudepot version numbers in lock-step across Cargo.toml, package.json, src-tauri/tauri.conf.json, README.md, the web install page, and stub a CHANGELOG section. Does not commit — leaves changes for review.
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

### Step 1 — Read the current version

Read `Cargo.toml` line `version = "..."` under `[workspace.package]`.
That's the authoritative current version. Parse as `CURRENT = X.Y.Z`.

Confirm all three files agree. If they don't, STOP and report the
drift — bumping from a drifted state would silently adopt one file's
opinion as canonical.

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

Do NOT rewrite `Cargo.lock` manually — `cargo check --workspace` will
regenerate it on the next build. Run `cargo check -p claudepot-cli`
to confirm the lockfile absorbs the change cleanly.

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
git diff --stat
git diff Cargo.toml package.json src-tauri/tauri.conf.json CHANGELOG.md README.md
```

### Step 6 — Do NOT commit

Leave the changes staged-but-uncommitted. Version bumps ride with a
release commit that usually also adjusts `CHANGELOG.md` prose and
maybe `dev-docs/`. The user drives that final edit pass.

End with a one-line summary:

```
Bumped CURRENT → NEXT. 5 files changed. Review + commit when ready.
```

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
- Refuse if the working tree is dirty in any of the five touched
  files (Cargo.toml, package.json, src-tauri/tauri.conf.json,
  CHANGELOG.md, README.md) — let the user commit or stash first so
  the bump diff is isolated.
- Do not touch any other file, and do not touch other parts of the
  five files. Specifically: in README.md only the status banner
  line is in scope; do not retouch install snippets, version
  strings in code blocks, or anything else. Version strings in doc
  examples (e.g. `dev-docs/*.md`) are intentionally pinned and must
  not drift with the bump.
- Do not run `cargo build --release` or `pnpm tauri build` — those
  are release-step work, not bump-step work.
