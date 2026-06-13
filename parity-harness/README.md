# CC-parity harness

Goal: make sure `claudepot_core::config_view::effective_settings::compute`
produces the same merged settings as CC's real `loadSettingsFromDisk`
across every edge case in `dev-docs/config-section-plan.md` §8.4.

> **Scope note.** The harness currently verifies the *settings merge*
> only. The MCP half of the config view
> (`config_view::effective_mcp`, the `getMcpConfigsByScope` port) has
> no fixture machinery yet — see §7 for the TODO. Until that lands,
> `effective_mcp`'s behavior is locked only by its Rust unit tests.

## 1. What runs today

```text
  cargo xtask verify-cc-parity [--only <fixture>]
```

For each subdirectory under `parity-harness/fixtures/`, the verifier:

1. Checks `notes.md` exists and cites the CC version pinned in
   `parity-harness/PINNED_CC_VERSION` (as `claude-code@<version>`).
   This makes a pin bump checkable: bumping the pin without
   re-deriving the fixtures fails the harness.
2. Reads `input.json` — the source bundle (plugin_base, user, project,
   local, flag, policy×4) in the shape the harness documents.
3. Feeds it to `effective_settings::compute_raw`.
4. Fails if `compute_raw` reports a merge divergence (the
   provenance-annotated merge disagreed with the plain CC-parity
   merge — a claudepot-core bug the plain-merge backstop would
   otherwise hide).
5. Diffs `merged` against `expected.json` and fails loudly on
   mismatch, printing each diverging key-path with both values.

The verifier is pure Rust: no network, no filesystem beyond
`parity-harness/**`. It runs in CI (`.github/workflows/ci.yml`,
lint job). Adding a fixture is just dropping a new directory.

## 2. Fixture layout

```
parity-harness/fixtures/case_NN_<name>/
├── input.json      # source bundle
├── expected.json   # merged output
└── notes.md        # REQUIRED — derivation rationale + CC source
                    # file/line refs + the pinned version string
                    # (claude-code@<PINNED_CC_VERSION>)
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
| `case_05_local_over_project` | Local layer overrides project and user at shared keys. |
| `case_06_flag_over_local` | Flag layer overrides local; non-overlapping local key retained. |
| `case_07_array_concat_dedupe` | Same array-of-primitives key in user+project: concat lower-then-upper, value-dedupe, first occurrence keeps position. |
| `case_08_hooks_concat_order` | Arrays of objects (hook entries) across layers: order preserved, equal-content duplicates kept (objects never deduped). |
| `case_09_policy_managed_file_winner` | Policy fallthrough reaches managed-file composite; populated HKCU below never consulted. |
| `case_10_policy_hkcu_winner` | HKCU wins when all higher policy origins are absent; still overrides non-policy layers. |
| `case_11_policy_empty_remote_skipped` | Present-but-empty (`{}`) remote source skipped without error; populated mdm_admin wins. |
| `case_12_scalar_clobber_empty_object_noop` | Higher scalar clobbers lower object wholesale; higher empty object is a no-op. |

## 4. Version pinning and the adapter gap

`parity-harness/PINNED_CC_VERSION` records — in one machine-readable
place — which CC version every `expected.json` was hand-derived from.
The verifier requires each fixture's `notes.md` to cite
`claude-code@<that version>`, so the pin can't drift away from the
goldens silently. **Bumping the pin means re-walking every fixture
against the new CC source and updating its notes.md.** Re-pin cadence:
treat a CC minor-version jump or a quarter — whichever comes first —
as the trigger to diff `utils/settings/settings.ts` between pinned and
current and re-derive anything that moved.

Plan §8.6 describes three candidate strategies for auto-regenerating
`expected.json` from a real CC tree. Today none of them work
out-of-the-box for the published CC source:

- **(a) CC-side shim inside the CC source tree.** `loadSettingsFromDisk`
  is not exported; `getSettingsWithErrors` is, but the raw source
  imports `bun:bundle` virtuals that only resolve through the bundler
  pipeline. A shim file must live inside the CC tree so the relative
  imports resolve, AND it must be loaded through the CC bundler (not
  via `bun run` on the raw source).
- **(b) Vendored fork.** Possible but high upkeep — every CC version
  bump needs a re-vendor.
- **(c) Behavioral probe.** Drive `claude -p 'noop'` in a fixture
  with a distinctive key at each scope, read the effective key via
  a hook that dumps it to a sentinel file. Works for spot-checks; too
  coarse to cover every §8.4 case.

Interim: `expected.json` files are **hand-authored** from a close
reading of `utils/settings/settings.ts` at the pinned version, with
the exact function + line refs recorded per fixture in `notes.md`.
The Rust `effective_settings::compute` is tested against those
goldens.

When a real adapter lands, `parity-harness/dump.ts` takes over:

```bash
# regenerate expected.json for a single fixture (FUTURE — stub today)
bun parity-harness/dump.ts parity-harness/fixtures/case_01_user_project_merge
```

Until then `dump.ts` prints the contract and exits 1, and the
verifier ignores `CLAUDE_SRC` (it warns if you set it — no code
consumes it yet).

## 5. Adding a new fixture

1. Pick a `case_NN_<name>` directory — increment NN from the highest
   existing case; names are lower_snake.
2. Put the source bundle in `input.json`.
3. Work through CC's `loadSettingsFromDisk` logic by hand for that
   bundle, reading the source at the pinned version. Write the merged
   result to `expected.json`.
4. Write `notes.md` (required — the verifier fails without it) with
   the CC source file + line refs and the literal string
   `claude-code@<PINNED_CC_VERSION>` so future maintainers can
   re-verify quickly.
5. `cargo xtask verify-cc-parity --only <name>` to check. If the
   Rust port matches, commit. If not, either the fixture is wrong or
   the port diverges — investigate before committing.

## 6. CI integration

`cargo xtask verify-cc-parity` exits non-zero on any mismatch, missing
or version-stale `notes.md`, or reported merge divergence. It is wired
into the lint job in `.github/workflows/ci.yml`:

```yaml
- name: CC-parity fixtures (cargo xtask verify-cc-parity)
  run: cargo xtask verify-cc-parity
```

The harness needs nothing beyond `cargo` and the workspace —
no CC source, no bun. When the adapter lands, CI can be augmented
to refresh goldens on schedule.

## 7. TODO — MCP parity (`getMcpConfigsByScope`)

`config_view::effective_mcp` ports CC's MCP config-by-scope logic
(project-chain deepest-wins, enterprise lockout, plugin
dedupe-by-content-hash, approval simulation across three modes) and
is **more** drift-prone against CC than the settings merge — but it
has zero harness coverage today. The harness claims parity for the
settings merge only.

Sketch for closing the gap (deliberately not started until someone
can derive the goldens from CC source with the same rigor as the
settings fixtures — the relevant CC surface spans
`src/services/mcp/config.ts` (`getMcpConfigsByScope`) plus the
approval flow in `src/services/mcpServerApproval.tsx`):

- Add a `kind` field to `input.json` (`"settings"` default,
  `"mcp"` new) or a sibling `mcp_input.json` fixture kind.
- A second code path in `run_fixture` feeds an `McpSourceBundle`
  into `effective_mcp::compute` and diffs the server set +
  approval states (skip the `masked` field — masking is
  Claudepot-side, not CC parity surface).
- Starter fixtures: project-chain deepest-wins, enterprise lockout,
  `enableAllProjectMcpServers` gating — each with a notes.md citing
  the CC source lines, same as the settings fixtures.
