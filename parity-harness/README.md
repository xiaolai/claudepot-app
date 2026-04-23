# CC-parity harness

Goal: make sure `claudepot_core::config_view::effective_settings::compute`
produces the same merged settings as CC's real
`loadSettingsFromDisk` / `getMcpConfigsByScope` across every edge
case in `dev-docs/config-section-plan.md` §8.4.

## 1. What runs today

```text
  cargo xtask verify-cc-parity [--only <fixture>]
```

For each subdirectory under `parity-harness/fixtures/`, the verifier:

1. Reads `input.json` — the source bundle (plugin_base, user, project,
   local, flag, policy×4) in the shape the harness documents.
2. Feeds it to `effective_settings::compute`.
3. Diffs `merged` against `expected.json`.
4. Fails loudly on mismatch (prints a per-line diff).

The verifier is pure Rust: no network, no filesystem beyond
`parity-harness/**`. It runs in CI. Adding a fixture is just dropping
a new directory.

## 2. Fixture layout

```
parity-harness/fixtures/case_NN_<name>/
├── input.json      # source bundle
├── expected.json   # merged output
└── notes.md        # (optional) rationale + CC source line refs
```

`input.json` top-level keys:

| Key | Shape | Notes |
|---|---|---|
| `plugin_base` | object or `null` | Lowest-precedence layer. |
| `user` | object or `null` | `~/.claude/settings.json`. |
| `project` | object or `null` | `<cwd>/.claude/settings.json`. |
| `local` | object or `null` | `settings.local.json`. |
| `flag` | object or `null` | `--settings` CLI flag (out of scope for Claudepot). |
| `policy` | array of 4 entries | `[remote, mdm_admin, managed_file_composite, hkcu_user]`. Each `{origin, value}`. First non-empty wins. |

`expected.json` is the post-merge object (secrets NOT masked — the
harness verifies the unmasked merge output so it's directly comparable
to what CC would produce before its own outbound serialization).

## 3. Current fixtures

| # | What it exercises |
|---|---|
| `case_01_user_project_merge` | Plain precedence: project overrides user at shared key, missing keys retained from lower layer. |
| `case_02_null_clobber` | `null` at higher precedence clobbers a populated container below. |
| `case_03_policy_remote_over_mdm` | Policy first-source-wins: remote wins even though mdm_admin is populated; non-policy scopes keep their non-overlapping keys. |
| `case_04_plugin_base_agent` | `plugin_base.agent` visible under user `theme`; project overrides `agent.model` via deep merge, retains plugin's `agent.tools`. |

## 4. The adapter gap (and how to close it)

Plan §8.6 describes three candidate strategies for auto-regenerating
`expected.json` from a real CC tree. Today none of them work
out-of-the-box for the published CC 2.1.88 source:

- **(a) CC-side shim inside `$CLAUDE_SRC`.** `loadSettingsFromDisk` is
  not exported; `getSettingsWithErrors` is, but the raw source
  imports `bun:bundle` virtuals that only resolve through the bundler
  pipeline. A shim file must live inside `$CLAUDE_SRC` so the relative
  imports resolve, AND it must be loaded through the CC bundler (not
  via `bun run` on the raw source).
- **(b) Vendored fork.** Possible but high upkeep — every CC version
  bump needs a re-vendor.
- **(c) Behavioral probe.** Drive `claude -p 'noop'` in a fixture
  with a distinctive key at each scope, read the effective key via
  a hook that dumps it to a sentinel file. Works for spot-checks; too
  coarse to cover every §8.4 case.

Interim: `expected.json` files are **hand-authored** from a close
reading of `utils/settings/settings.ts:645-790` (lines pinned to
`claude-code@2.1.88`). The Rust `effective_settings::compute` is
tested against those goldens.

When a real adapter lands, `parity-harness/dump.ts` takes over:

```bash
# regenerate expected.json for a single fixture
bun parity-harness/dump.ts parity-harness/fixtures/case_01_user_project_merge

# CI re-verifies goldens against a live CC tree
CLAUDE_SRC=~/github/claude_code_src cargo xtask verify-cc-parity
```

Until then `dump.ts` prints the contract and exits 1.

## 5. Adding a new fixture

1. Pick a `case_NN_<name>` directory — increment NN from the highest
   existing case; names are lower_snake.
2. Put the source bundle in `input.json`.
3. Work through CC's `loadSettingsFromDisk` logic by hand for that
   bundle. Write the merged result to `expected.json`.
4. Optionally drop a `notes.md` with CC source line refs so future
   maintainers can re-verify quickly.
5. `cargo xtask verify-cc-parity --only <name>` to check. If the
   Rust port matches, commit. If not, either the fixture is wrong or
   the port diverges — investigate before committing.

## 6. CI integration

`cargo xtask verify-cc-parity` exits non-zero on any mismatch, so
wiring it to CI is a one-liner:

```yaml
- name: Verify CC parity
  run: cargo xtask verify-cc-parity
```

The harness needs nothing beyond `cargo` and the workspace —
no CC source, no bun. When the adapter lands, CI can be augmented
to refresh goldens on schedule.
