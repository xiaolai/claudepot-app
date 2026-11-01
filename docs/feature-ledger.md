# Feature ledger

One row per user-facing feature: where it is reached, the code that
implements it, the user doc that explains it, and the first release
that shipped it. It answers "is feature X documented, and where?"
without a search across the repo.

`cargo xtask verify-docs` checks this table against the source
(`crates/xtask/src/feature_ledger.rs`). It fails when:

- a GUI section in `src/sections/registry.tsx`, a Settings pane in
  `src/sections/settings/panes.ts`, or a visible top-level CLI verb has
  no row naming it in **Reached from**;
- a web feature page under `web/src/app/(reader)/app/features/` is
  named by no row;
- a **Code** path does not exist;
- a **Docs** page does not exist, or names a heading the page does not
  have;
- a **Since** version has no `## X.Y.Z` section in `CHANGELOG.md`.

It cannot tell whether a doc is *good*, or whether a feature with no
surface of its own (a background behaviour) is missing a row. Those
stay with review.

## Columns

- **Reached from** — `section:<id>` (registry id), `pane:<id>` (Settings
  pane id), `cli:<verb>` (top-level CLI verb), or `background` for
  behaviour with no control of its own. A row may list several.
- **Code** — repo-relative files or directories; the main one first.
- **Docs** — `` `<page>` § <heading> ``, where `<page>` is a web feature
  page by directory name (`accounts` is
  `web/src/app/(reader)/app/features/accounts/page.mdx`) or a
  repo-relative path such as `README.md`, and the heading is optional.
  `none` marks a gap: the feature ships with no user doc.
- **Since** — the first release that shipped the main code path: the
  first workspace version bump in `Cargo.toml` whose history contains
  the commit that added it, following renames. Tags are no help here —
  `release.yml` deletes old ones from origin. A directory is dated by
  its `mod.rs`, and `--follow` does not survive a directory rename
  (`agent/` was `automations/`), so date a renamed module by its old
  path. A feature that grew out of an older module reads as younger
  than it is. Set it once, when the row is added.

## The ledger

| Feature | Reached from | Code | Docs | Since |
|---|---|---|---|---|
| Account registry: add, remove, inspect | `section:accounts`, `cli:account` | `crates/claudepot-core/src/account.rs`, `src/sections/accounts` | `accounts` § Add an account | 0.0.2 |
| Identity verification | `section:accounts`, `cli:account` | `crates/claudepot-core/src/account_verification.rs`, `crates/claudepot-core/src/services/identity.rs` | `accounts` § Verify | 0.0.2 |
| CLI slot: switch the account Claude Code uses | `section:accounts`, `cli:cli` | `crates/claudepot-core/src/cli_backend`, `crates/claudepot-core/src/services/cli_service.rs` | `accounts` § Switch slots | 0.0.2 |
| Ground-truth sign-in status | `cli:status` | `crates/claudepot-cli/src/commands/status.rs` | `global` § CLI mirror | 0.0.2 |
| Desktop slot: switch the account Claude Desktop uses | `section:accounts`, `cli:desktop` | `crates/claudepot-core/src/desktop_backend`, `crates/claudepot-core/src/services/desktop_service.rs` | `accounts` § Switch slots | 0.0.2 |
| Claude Desktop's moved Windows data folder | `background` | `crates/claudepot-core/src/desktop_backend/windows_data_dir.rs` | `accounts` § Where ClauDepot finds Claude Desktop | 0.7.1 |
| Rate limits per account | `section:accounts`, `cli:usage` | `crates/claudepot-core/src/services/usage_cache.rs`, `crates/claudepot-core/src/services/usage_snapshot.rs` | `accounts` § Rate limits at a glance | 0.0.2 |
| Local token and cost rollup | `cli:usage` | `crates/claudepot-core/src/usage_local.rs`, `crates/claudepot-core/src/pricing` | `README.md` | 0.0.10 |
| Proactive token refresh | `background` | `crates/claudepot-core/src/token_refresh.rs` | `accounts` § Keeping stored accounts signed in | 0.2.12 |
| Live session strip | `section:events` | `crates/claudepot-core/src/session_live`, `src/shell/SidebarLiveStrip.tsx` | `activities` § Live strip | 0.0.2 |
| Today and month cost dashboard | `section:events` | `src/sections/events`, `src/costs.ts` | `activities` § Today / month dashboard | 0.0.6 |
| Activity cards: anomalies and milestones | `section:events`, `cli:activity` | `crates/claudepot-core/src/activity` | `activities` § Cards stream | 0.0.6 |
| Waiting-session notifications | `pane:notifications` | `crates/claudepot-core/src/notifications.rs`, `crates/claudepot-core/src/notification_log.rs` | `settings` § Notifications | 0.1.28 |
| Project and session browser | `section:projects`, `cli:project`, `cli:session` | `crates/claudepot-core/src/session_index`, `src/sections/projects` | `projects` § Master-detail | 0.0.2 |
| Project rename and move, journaled | `section:projects`, `cli:project` | `crates/claudepot-core/src/project/core.rs`, `crates/claudepot-core/src/project/journal.rs` | `projects` § What you can do | 0.0.2 |
| Orphaned sessions: adopt, move, clean | `section:projects`, `cli:session` | `crates/claudepot-core/src/session/move_` | `projects` § Orphans | 0.0.2 |
| Worktree grouping | `section:projects` | `crates/claudepot-core/src/session/worktree.rs` | `projects` § Worktrees | 0.0.2 |
| Permission grants | `section:projects` | `crates/claudepot-core/src/permission` | `projects` § Permission grants | 0.1.34 |
| Per-project environment files | `section:projects` | `src/sections/projects/ProjectEnvPanel.tsx` | `projects` § Environment files | 0.1.34 |
| Machine-to-machine migration bundles | `section:projects`, `cli:export`, `cli:import`, `cli:migrate` | `crates/claudepot-core/src/migrate` | `README.md` | 0.0.6 |
| Peer messaging between sessions | `background` | `crates/claudepot-core/src/peer` | `projects` § Peer messaging | 0.5.0 |
| Memory files: CLAUDE.md and memory change log | `section:projects`, `cli:memory` | `crates/claudepot-core/src/memory_view.rs`, `crates/claudepot-core/src/memory_log.rs` | `global` § Memory | 0.1.4 |
| Shared memory: transcript search, memories, decisions | `section:shared-memory` | `crates/claudepot-core/src/shared_memory`, `src/sections/knowledge` | `memory` § The four tabs | 0.1.35 |
| Lessons: distill, review, bind | `section:shared-memory`, `cli:lesson` | `crates/claudepot-core/src/shared_memory/distill.rs`, `crates/claudepot-core/src/shared_memory/review.rs` | `memory` § Finding what's worth keeping | 0.2.5 |
| Cross-machine corpus and repetition detectors | `cli:corpus` | `crates/claudepot-core/src/corpus` | `memory` § Beyond this machine | 0.3.4 |
| MCP memory server and agent snippet | `pane:mcp`, `cli:mcp` | `crates/claudepot-core/src/mcp_snippet.rs`, `crates/claudepot-core/src/mcp_probe.rs` | `memory` § The MCP server | 0.1.36 |
| Codex transcript indexing | `cli:codex` | `crates/claudepot-core/src/codex_session` | `memory` § Indexing Codex transcripts | 0.1.35 |
| API key and OAuth token inventory | `section:keys` | `crates/claudepot-core/src/keys` | `keys` § Add a key | 0.0.2 |
| Env secret vault | `section:keys` | `crates/claudepot-core/src/env_vault` | `keys` § Secret vault | 0.1.34 |
| Third-party provider routes | `section:third-party` | `crates/claudepot-core/src/routes` | `third-parties` § Add a route | 0.0.6 |
| Scheduled agents | `section:automations`, `cli:agent` | `crates/claudepot-core/src/agent` | `automations` § Anatomy | 0.0.6 |
| Agent templates | `section:automations` | `crates/claudepot-core/src/templates` | `automations` § Templates | 0.1.1 |
| Config cascade view | `section:config` | `crates/claudepot-core/src/config_view` | `global` § Config | 0.0.6 |
| CLAUDE.md health | `section:config` | `crates/claudepot-core/src/memory_health.rs` | `global` § Memory | 0.1.1 |
| Claude Code tips ledger | `section:config` | `crates/claudepot-core/src/cc_tips` | `global` § Tips | 0.1.20 |
| Claude Code and Desktop updates | `section:config`, `cli:update` | `crates/claudepot-core/src/updates` | `global` § Updates | 0.0.17 |
| Claude Code environment variables | `section:config` | `crates/claudepot-core/src/cc_env`, `src/sections/config/envvars` | `global` § Claude Code environment variables | 0.3.5 |
| Disable and trash CC artifacts | `section:config`, `pane:cleanup` | `crates/claudepot-core/src/artifact_lifecycle` | `global` § Disabling and trashing artifacts | 0.0.6 |
| Auto-memory and auto-dream toggles | `section:config`, `cli:settings` | `crates/claudepot-core/src/auto_dream.rs`, `src/sections/global/AutoMemoryGlobalCard.tsx` | `global` § Auto-memory and background consolidation | 0.2.16 |
| Boards: agent-written dashboards | `section:boards`, `cli:experimental` | `crates/claudepot-core/src/board` | `boards` § What a board is | 0.3.7 |
| General preferences | `pane:general` | `src/sections/SettingsSection.tsx` | `settings` § General | 0.0.2 |
| Claude Code model toggles: fast mode, thinking, models, attribution | `pane:general` | `crates/claudepot-core/src/fast_mode_toggle.rs`, `crates/claudepot-core/src/thinking_toggle.rs`, `crates/claudepot-core/src/available_models.rs`, `crates/claudepot-core/src/attribution_settings.rs` | `settings` § Claude Code model settings | 0.3.0 |
| Appearance and theme | `pane:appearance` | `src/styles/tokens.css` | `settings` § Appearance | 0.0.2 |
| Network status | `pane:network` | `crates/claudepot-core/src/service_status.rs` | `settings` § Network | 0.1.3 |
| Auto-rotation | `pane:rotation` | `crates/claudepot-core/src/rotation` | `settings` § Rotation | 0.1.22 |
| Remote control from a phone | `pane:remote`, `cli:remote` | `crates/claudepot-core/src/remote`, `panel` | `settings` § Remote | 0.5.0 |
| Remote quick prompts | `pane:remote` | `crates/claudepot-core/src/quick_prompt`, `src/sections/settings/QuickPromptsPane.tsx` | `settings` § Remote | 0.5.0 |
| Subscription relay | `pane:relay`, `cli:relay` | `crates/claudepot-core/src/relay` | `settings` § Relay | 0.7.0 |
| Transcript retention | `pane:retention` | `crates/claudepot-core/src/cc_retention.rs` | `settings` § Retention | 0.3.4 |
| Claude Code health (`claude doctor`) | `pane:health` | `crates/claudepot-core/src/cc_doctor` | `settings` § Health | 0.1.23 |
| Cleanup: prune, slim, trash | `pane:cleanup` | `crates/claudepot-core/src/session/prune.rs`, `crates/claudepot-core/src/session/slim.rs`, `crates/claudepot-core/src/trash.rs` | `settings` § Cleanup | 0.0.3 |
| Protected paths | `pane:protected` | `crates/claudepot-core/src/protected_paths.rs` | `settings` § Protected paths | 0.0.2 |
| Session export to GitHub | `pane:github` | `crates/claudepot-core/src/session/export_delivery.rs` | `settings` § GitHub | 0.0.6 |
| Stale lock recovery | `pane:locks` | `crates/claudepot-core/src/project/lock.rs` | `settings` § Locks | 0.0.2 |
| Claudepot self-check and logs | `pane:diagnostics`, `cli:doctor`, `cli:logs` | `crates/claudepot-core/src/services/doctor_service.rs`, `crates/claudepot-core/src/diagnostic_logging.rs` | `settings` § Diagnostics | 0.0.2 |
| About and app updates | `pane:about` | `crates/claudepot-core/src/release_channel.rs` | `settings` § About | 0.1.40 |
| Settings | `section:settings` | `src/sections/SettingsSection.tsx`, `src/sections/settings/panes.ts` | `settings` | 0.0.2 |
