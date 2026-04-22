---
description: Bump Claudepot version numbers in lock-step across Cargo.toml, package.json, src-tauri/tauri.conf.json, and stub a CHANGELOG section. Does not commit — leaves changes for review.
---

# Bump version

Bump Claudepot's version in every file that holds one. Version lives
in three sources of truth:

| File | Line | Notes |
|---|---|---|
| `Cargo.toml` | top-level `[workspace.package] version = "X.Y.Z"` | Feeds every Rust crate via `version.workspace = true` |
| `package.json` | `"version": "X.Y.Z"` | Frontend build stamp |
| `src-tauri/tauri.conf.json` | `"version": "X.Y.Z"` | Shown to the OS (menu bar "About", bundle metadata) |

All three MUST match byte-for-byte. A mismatch produces a release with
a wrong "About" dialog or bundles that refuse to install over previous
versions.

## Inputs

`$ARGUMENTS` must be one of:

- `patch` — bump the last segment (`0.0.2` → `0.0.3`)
- `minor` — bump the middle segment, zero the last (`0.0.2` → `0.1.0`)
- `major` — bump the first segment, zero the rest (`0.1.0` → `1.0.0`)
- An explicit `X.Y.Z` string — used verbatim

Reject anything else. Do NOT accept pre-release suffixes (`-alpha`,
`-rc.1`) — Claudepot's versioning scheme uses the numeric tier as the
release stage (see `CHANGELOG.md` header):

- `0.0.x` — alpha
- `0.1.x` — beta
- `1.0.0+` — stable

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

Store as `NEXT`.

### Step 3 — Apply edits

Edit exactly these three locations:

1. `Cargo.toml` → the `version = "CURRENT"` line under
   `[workspace.package]`. Use `Edit` with the full surrounding line to
   avoid accidentally rewriting a crate's dev-dep version pin.
2. `package.json` → the top-level `"version": "CURRENT"` field. Keep
   the surrounding JSON formatting (2-space indent, trailing comma
   where present).
3. `src-tauri/tauri.conf.json` → the top-level `"version"` field.

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
git diff Cargo.toml package.json src-tauri/tauri.conf.json CHANGELOG.md
```

### Step 6 — Do NOT commit

Leave the changes staged-but-uncommitted. Version bumps ride with a
release commit that usually also adjusts `CHANGELOG.md` prose and
maybe `dev-docs/`. The user drives that final edit pass.

End with a one-line summary:

```
Bumped CURRENT → NEXT. 4 files changed. Review + commit when ready.
```

## Rules

- One argument only. Reject `$ARGUMENTS` that combines `patch` with a
  literal version, or adds pre-release suffixes.
- Refuse to bump backwards (`NEXT <= CURRENT`). If the user really
  wants that, they can edit the files directly.
- Refuse if the working tree is dirty in any of the four touched
  files — let the user commit or stash first so the bump diff is
  isolated.
- Do not touch any other file. Version strings in doc examples (e.g.
  `dev-docs/*.md`) are intentionally pinned and must not drift with
  the bump.
- Do not run `cargo build --release` or `pnpm tauri build` — those
  are release-step work, not bump-step work.
