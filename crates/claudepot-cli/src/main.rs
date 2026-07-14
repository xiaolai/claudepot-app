use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use claudepot_core::account::AccountStore;
use claudepot_core::paths;
use claudepot_core::services::usage_cache::UsageCache;

mod clipboard;
mod commands;
mod output;
mod time_fmt;

#[derive(Parser)]
#[command(
    name = "claudepot",
    // Pulled from `version` in the workspace's root `Cargo.toml` via
    // CARGO_PKG_VERSION. Bumps land in lock-step across `Cargo.toml`,
    // `package.json`, and `tauri.conf.json` (see the `bump` skill);
    // wiring this through env! keeps the CLI in sync automatically.
    version = env!("CARGO_PKG_VERSION"),
    about = "Multi-account Claude Code / Desktop switcher"
)]
struct Cli {
    /// Output JSON instead of human-readable text
    #[arg(long, short, global = true)]
    json: bool,

    /// Suppress informational messages
    #[arg(long, short, global = true)]
    quiet: bool,

    /// Enable debug logging
    #[arg(long, short, global = true)]
    verbose: bool,

    /// Skip confirmation prompts
    #[arg(long, short, global = true)]
    yes: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage registered accounts
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Claude Code CLI credential management
    Cli {
        #[command(subcommand)]
        action: CliAction,
    },
    /// Claude Desktop session management
    Desktop {
        #[command(subcommand)]
        action: DesktopAction,
    },
    /// Manage CC project session storage
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Draft, list, and inspect agents (scheduled `claude -p`
    /// runs). `agent draft` lets an AI client *propose* an agent;
    /// `list` / `show` read the store. Arming a draft — and any
    /// edit of an armed agent — is human-only in the Claudepot
    /// GUI; this CLI deliberately has no `install` or `edit` verb.
    /// The hidden `_record-run` plumbing verb is invoked by the
    /// per-agent helper shim.
    //
    // `alias = "automation"` keeps already-installed agent shims
    // (which call `claudepot automation _record-run …`) working
    // across the Phase 1 rename.
    #[command(name = "agent", alias = "automation")]
    Agent {
        // Boxed: `AgentAction::Draft` carries ~16 optional flags,
        // which would otherwise make `Commands` itself a
        // large-variant enum. The box keeps the `Commands`
        // discriminant small; the enum is parsed once at startup.
        #[command(subcommand)]
        action: Box<AgentAction>,
    },
    /// Export the current project's CC state to a portable bundle
    /// (`*.claudepot.tar.zst`).
    ///
    /// Lives at the top level (not under `project`) so the call sites
    /// in scripting / SSH paths stay short. Internally a thin wrapper
    /// over `claudepot-core::migrate::export_projects`.
    #[command(name = "export")]
    Export {
        #[command(flatten)]
        args: commands::project_migrate::ExportArgs,
    },
    /// Import a `*.claudepot.tar.zst` bundle into this machine.
    #[command(name = "import")]
    Import {
        #[command(flatten)]
        args: commands::project_migrate::ImportArgs,
    },
    /// Inspect or manage migration bundles.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Manage CC session transcripts (move between projects, rescue orphans)
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Inspect "what just happened" across CC sessions — anomaly + milestone cards
    Activity {
        #[command(subcommand)]
        action: ActivityAction,
    },
    /// Per-project memory artifacts: list, view, and inspect the
    /// change log for `CLAUDE.md` + `~/.claude/projects/<slug>/memory/*.md`.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Read or modify CC settings that Claudepot exposes as toggles
    /// (currently: auto-memory).
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Health check and diagnostics
    Doctor,
    /// Diagnostic log file controls.
    ///
    /// Claudepot's Tauri GUI writes every `tracing` event and any
    /// panic to a rolling daily log at the OS-standard log
    /// directory (`~/Library/Logs/com.claudepot.app/` on macOS).
    /// This subcommand resolves the directory path, optionally
    /// opens it in the OS file manager, or tails the current log.
    Logs {
        /// Open the log directory in the OS file manager.
        /// Mutually exclusive with `--tail` (tail never returns,
        /// so a combined invocation would silently drop the open).
        #[arg(long, conflicts_with = "tail")]
        open: bool,
        /// Follow the current `claudepot.log` (tail -f equivalent).
        #[arg(long, short = 'f')]
        tail: bool,
    },
    /// Ground-truth authentication status.
    ///
    /// Reads CC's shared credential slot, calls `/api/oauth/profile`,
    /// compares the verified email to Claudepot's `active_cli` pointer.
    /// Prints MATCH / DRIFT / NOT SIGNED IN. Exit code: 0 match,
    /// 2 drift, 3 couldn't check.
    Status,
    /// Local cost report — token totals + USD cost rolled up by
    /// project, derived from CC transcripts on disk.
    ///
    /// No network call. Cost computed against the bundled price
    /// table; per-account attribution is intentionally omitted (CC
    /// transcripts don't carry an account id, and claudepot doesn't
    /// keep a swap-event log to reconstruct it).
    Usage {
        #[command(subcommand)]
        action: UsageAction,
    },
    /// Manage Claude Code CLI and Claude Desktop updates.
    ///
    /// Surfaces detected installs, probes upstream for the latest
    /// version, and (on request) drives an install. CC's native
    /// installer auto-updates in the background; this verb is for
    /// manual checks and for forcing the update right now. Desktop
    /// installs route through Homebrew Cask when brew-managed,
    /// direct .zip download otherwise. See `dev-docs/auto-updates.md`.
    Update {
        #[command(subcommand)]
        action: UpdateAction,
    },
    /// Run an MCP server backed by Claudepot's shared memory.
    ///
    /// The memory-server subcommand starts a stdio MCP server that
    /// exposes search / read / remember / log-decision /
    /// submit-evidence tools to Claude Code and Codex. See
    /// `dev-docs/codex-plans/20260515-1130-shared-memory.md` (WI-008)
    /// for the protocol shape and `dev-docs/reports/rmcp-spike-2026-05-15.md`
    /// for the SDK verdict.
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Manage Codex rollout transcripts.
    ///
    /// Codex stores rollout JSONL under `$CODEX_HOME/sessions/`
    /// (default `~/.codex/sessions/`). The `index` subcommand
    /// walks the tree and populates Claudepot's cross-harness
    /// `exchanges` table so `claudepot mcp memory-server` can
    /// surface Codex transcripts via `claudepot_search_memory` /
    /// `claudepot_read_conversation`.
    Codex {
        #[command(subcommand)]
        action: CodexAction,
    },
}

#[derive(Subcommand)]
enum McpAction {
    /// Run the Claudepot memory MCP server over stdio. Stdout is
    /// reserved for JSON-RPC frames; logs go to stderr.
    MemoryServer {
        /// Override the sessions.db path. Defaults to
        /// `~/.claudepot/sessions.db`.
        #[arg(long)]
        db: Option<std::path::PathBuf>,
    },
    /// Print the recommended Claude Code / Codex agent
    /// instruction snippet to stdout. The snippet tells the
    /// agent when to call the MCP tools (search before
    /// asking, remember durable facts, log decisions, submit
    /// evidence). Use this when you want to paste the content
    /// into your AGENTS.md / CLAUDE.md manually.
    PrintSnippet,
    /// Write the agent instruction snippet to a managed file
    /// (default `~/.claude/claudepot-mcp-instructions.md`). The
    /// user adds a single `@include` line to their CLAUDE.md /
    /// AGENTS.md — narrow edit surface, the file can be
    /// regenerated safely.
    InstallSnippet {
        /// Override the output path.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Print the recommended `@include` line for CLAUDE.md
        /// / AGENTS.md after writing.
        #[arg(long, default_value_t = true)]
        print_include: bool,
    },
}

#[derive(Subcommand)]
enum CodexAction {
    /// Index Codex rollouts into `sessions.db`. Idempotent — files
    /// whose (size, mtime, inode) tuple is unchanged since the last
    /// run are skipped.
    Index {
        /// Override the Codex sessions root. Defaults to
        /// `$CODEX_HOME/sessions/` (typically `~/.codex/sessions/`).
        #[arg(long)]
        codex_home: Option<std::path::PathBuf>,
        /// Override the sessions.db path. Defaults to
        /// `~/.claudepot/sessions.db`.
        #[arg(long)]
        db: Option<std::path::PathBuf>,
    },
    /// Force a full rebuild of the transcript-derived tables
    /// (`sessions`, `session_turns`, `exchanges`, `tool_calls`,
    /// `exchange_fts`) on the next open. Sets the `_pending_rescan`
    /// marker; the next `claudepot codex index` (or any
    /// `SessionIndex::open`) clears the cache atomically inside the
    /// migration transaction and the next refresh repopulates
    /// from disk.
    ///
    /// Preserves durable rows (`memories`, `decisions`,
    /// `evidence_records`, `memory_links`).
    Rebuild {
        /// Override the sessions.db path. Defaults to
        /// `~/.claudepot/sessions.db`.
        #[arg(long)]
        db: Option<std::path::PathBuf>,
    },
    /// Wipe Shared Memory rows (both transcript-derived and
    /// durable). Use to remove on-disk copies of unredacted
    /// transcript text after a contractor session, or to start
    /// over. Leaves the v4 schema in place. After this runs, the
    /// next `claudepot codex index` will repopulate the
    /// transcript-derived tables from disk; durable
    /// memories/decisions/evidence will be empty until you
    /// re-create them.
    ///
    /// Requires the global `--yes` to confirm — this is the only
    /// destructive path in the codex subcommand tree. Without it
    /// the command refuses and prints the row counts it would
    /// have removed.
    Forget {
        /// Override the sessions.db path. Defaults to
        /// `~/.claudepot/sessions.db`.
        #[arg(long)]
        db: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum UsageAction {
    /// Print the per-project cost + token report.
    Report {
        /// Time window: `all` (default) or `<n>d` (e.g. `7d`, `30d`).
        #[arg(long, default_value = "all")]
        window: String,
    },
}

#[derive(Subcommand)]
enum UpdateAction {
    /// Detect installs and probe upstream for the latest version
    /// (CC + Desktop). Persists the probe result to
    /// `~/.claudepot/updates.json`.
    Check,
    /// Force-update the active CC CLI install now (runs `claude
    /// update`). Refuses if `DISABLE_UPDATES=1` is set in CC's
    /// settings.json.
    Cli,
    /// Force-update Claude Desktop now. Refuses if Desktop is
    /// running. Routes through `brew upgrade --cask claude` when
    /// brew-managed, direct .zip + codesign-verified install
    /// otherwise.
    Desktop,
    /// Show or modify update settings. With no flags, prints the
    /// current configuration. Each flag triggers exactly one write.
    Config {
        #[command(flatten)]
        args: commands::update::config::ConfigArgs,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List projects whose cwd no longer exists on disk (adoption candidates).
    ListOrphans,
    /// Move a single session transcript from one project cwd to another.
    Move {
        /// Session UUID (the `.jsonl` filename without extension).
        session_id: String,
        /// Current cwd this session lives under.
        #[arg(long)]
        from: String,
        /// Target cwd to move the session to.
        #[arg(long)]
        to: String,
        /// Proceed even if the source JSONL was modified recently
        /// (treated as potentially live).
        #[arg(long)]
        force_live: bool,
        /// Proceed even if a Syncthing `.sync-conflict-*` sibling exists.
        #[arg(long)]
        force_conflict: bool,
        /// Remove the source project dir if it is empty after the move.
        #[arg(long)]
        cleanup_source: bool,
    },
    /// Move every session under an orphaned slug into a live target cwd.
    AdoptOrphan {
        /// The slug directory name under `~/.claude/projects/`.
        slug: String,
        /// Live target cwd to adopt into.
        #[arg(long)]
        target: String,
    },
    /// Truncate the persistent session-index cache at
    /// `~/.claudepot/sessions.db`. Next list/GUI refresh rebuilds it
    /// from scratch. Safe — no transcripts or credentials are touched.
    RebuildIndex,
    /// Backfill `exchanges` + `tool_calls` rows for every indexed
    /// Claude transcript. Required for cross-harness search to
    /// surface Claude content via `claudepot mcp memory-server` —
    /// the existing session_index::refresh writes `sessions` rows
    /// but doesn't populate the exchange-level tables. Idempotent;
    /// safe to re-run.
    ///
    /// Run `claudepot session rebuild-index` first (or wait for any
    /// list/GUI refresh) so the `sessions` rows are present.
    BackfillExchanges {
        /// Override `CLAUDE_CONFIG_DIR`. Defaults to the same
        /// resolution Claude Code uses.
        #[arg(long)]
        claude_config: Option<std::path::PathBuf>,
    },
    /// Inspect one transcript: classification, chunks, linked tool
    /// calls, subagents, phases, context attribution. Read-only.
    View {
        /// Session UUID (filename stem) OR absolute `.jsonl` path.
        target: String,
        /// Render mode. `summary` prints human-readable pieces;
        /// `chunks|tools|classify|subagents|phases|context` emit the
        /// JSON payload a GUI or script would consume.
        #[arg(long, default_value = "summary", value_parser = ["summary", "chunks", "tools", "classify", "subagents", "phases", "context"])]
        show: String,
    },
    /// Export a session transcript. Redacts sk-ant-* tokens by default.
    Export {
        #[command(flatten)]
        args: commands::session::ExportArgs,
    },
    /// Cross-session text search. Scans first-user-prompts and
    /// assistant/user turns case-insensitively.
    Search {
        /// Query string (case-insensitive, ≥2 chars).
        query: String,
        /// Maximum hits to return.
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Group sessions by git repository (collapses worktrees).
    Worktrees,
    /// Bulk delete session transcripts into the reversible trash.
    /// Dry-run by default — pass `--execute` to actually move files.
    Prune {
        #[command(flatten)]
        args: commands::session::PruneArgs,
    },
    /// Reversible trash for prune/slim operations.
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// Rewrite a transcript, dropping oversized tool_result payloads
    /// and, optionally, base64 image/document payloads.
    /// Dry-run by default — pass `--execute` to rewrite in place.
    Slim {
        #[command(flatten)]
        args: commands::session::SlimArgs,
    },
}

#[derive(Subcommand)]
enum ActivityAction {
    /// Show recent activity cards (anomalies + milestones), newest first.
    Recent {
        #[command(flatten)]
        args: commands::activity::RecentArgs,
    },
    /// Walk every JSONL under `~/.claude/projects/` and rebuild the
    /// activity index. Idempotent — re-running adds zero rows when
    /// the source hasn't changed.
    Reindex,
}

#[derive(Subcommand)]
enum MemoryAction {
    /// List memory files for a project (defaults to cwd).
    List {
        /// Project root path. Defaults to the current directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Print a memory file's content. FILE may be an absolute path
    /// or a basename inside the project's memory dir or CLAUDE.md
    /// candidate locations.
    View {
        file: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Show recent change-log entries.
    Log {
        #[arg(long)]
        project: Option<String>,
        /// Limit to one file (absolute path or memory-dir basename).
        #[arg(long)]
        file: Option<String>,
        /// Maximum rows. Defaults to 50, capped at 10 000.
        #[arg(long)]
        limit: Option<usize>,
        /// Print the unified diff for each entry.
        #[arg(long)]
        show_diff: bool,
    },
}

#[derive(Subcommand)]
enum SettingsAction {
    /// Inspect or change CC's `autoMemoryEnabled` setting.
    AutoMemory {
        #[command(subcommand)]
        action: AutoMemoryAction,
    },
}

#[derive(Subcommand)]
enum AutoMemoryAction {
    /// Show the effective state and per-source breakdown.
    Status {
        #[arg(long)]
        project: Option<String>,
    },
    /// Set `autoMemoryEnabled = true`. Without `--project`, writes
    /// to `~/.claude/settings.json` (global). With `--project`,
    /// writes to `<PROJECT>/.claude/settings.local.json`.
    Enable {
        #[arg(long)]
        project: Option<String>,
        /// Apply at project scope (writes to `.claude/settings.local.json`).
        #[arg(long = "project-scope")]
        project_scope: bool,
    },
    /// Set `autoMemoryEnabled = false`. Same scope rules as `enable`.
    Disable {
        #[arg(long)]
        project: Option<String>,
        #[arg(long = "project-scope")]
        project_scope: bool,
    },
    /// Remove the `autoMemoryEnabled` key from the relevant settings
    /// file so the next-higher layer takes over the decision.
    Clear {
        #[arg(long)]
        project: Option<String>,
        #[arg(long = "project-scope")]
        project_scope: bool,
    },
}

#[derive(Subcommand)]
enum TrashAction {
    /// List current trash batches.
    List {
        /// Only show entries older than the given duration.
        #[arg(long)]
        older_than: Option<String>,
    },
    /// Restore a trash batch by its id.
    Restore {
        /// Batch id (from `trash list`).
        id: String,
        /// Override destination cwd (parent dir) instead of the original.
        #[arg(long)]
        to: Option<String>,
    },
    /// Empty the trash. Honors the global `--yes` when on a TTY.
    Empty {
        /// Only empty entries older than the given duration.
        #[arg(long)]
        older_than: Option<String>,
    },
}

// The `Draft` variant carries ~16 optional clap flags, so it is
// much larger than the `_record-run` plumbing variant. This enum is
// parsed exactly once at process start and immediately destructured
// — it is never stored in bulk — so the size-difference cost the
// lint warns about does not apply here.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum AgentAction {
    /// Draft an inert agent from a spec. Accepts Claudepot-native
    /// JSON or `AgentDefinition`-shaped JSON via `--from-json`
    /// (file path, or `-` for stdin), and/or `--name/--cwd/--prompt`
    /// flags. The new agent has `lifecycle = draft`: it sits in
    /// `agents.json`, no scheduler artifact is created, NOTHING
    /// fires. Arming a draft is a human-only action in the
    /// Claudepot GUI — there is deliberately no `install` verb.
    Draft {
        /// Spec JSON source: a file path, or `-` to read stdin.
        /// Accepts both Claudepot-native and `AgentDefinition`
        /// shapes; the persisted form is always Claudepot-native.
        #[arg(long = "from-json", value_name = "FILE|-")]
        from_json: Option<String>,
        /// Agent name (a-z, 0-9, dash; 1-64). Required for a
        /// flags-only draft and for any `AgentDefinition`-shaped
        /// JSON (which carries no Claudepot name). Overrides the
        /// JSON `name` when both are present.
        #[arg(long)]
        name: Option<String>,
        /// Working directory the agent runs in. Required for a
        /// flags-only draft and for `AgentDefinition`-shaped JSON.
        #[arg(long)]
        cwd: Option<String>,
        /// The `claude -p` prompt. Required for a flags-only draft.
        #[arg(long)]
        prompt: Option<String>,
        /// Optional human-friendly display name.
        #[arg(long = "display-name")]
        display_name: Option<String>,
        /// Optional one-line description.
        #[arg(long)]
        description: Option<String>,
        /// Model id (e.g. `claude-haiku-4-5`). Empty = CLI default.
        #[arg(long)]
        model: Option<String>,
        /// Permission mode: default | acceptEdits | bypassPermissions
        /// | dontAsk | plan | auto.
        #[arg(long = "permission-mode")]
        permission_mode: Option<String>,
        /// Comma-/space-separated allowed-tools whitelist.
        #[arg(long = "allowed-tools")]
        allowed_tools: Option<String>,
        /// Comma-/space-separated disallowed-tools list.
        #[arg(long = "disallowed-tools")]
        disallowed_tools: Option<String>,
        /// Five-field cron expression. Sets a cron trigger; absent
        /// = a manual trigger (the safe default for a draft).
        #[arg(long)]
        cron: Option<String>,
        /// IANA timezone for the cron trigger. Requires `--cron`.
        #[arg(long)]
        timezone: Option<String>,
        /// Pin the agent to a specific account email. Empty =
        /// the CLI-active account at fire time.
        #[arg(long = "run-as")]
        run_as: Option<String>,
        /// Per-run token ceiling.
        #[arg(long = "task-budget")]
        task_budget: Option<u64>,
        /// Attach Claudepot's own MCP memory server to the draft.
        #[arg(long = "attach-memory")]
        attach_memory: bool,
        /// Audit actor id recorded as `drafted_by` (e.g.
        /// `claude-code@2026-05-22`). Defaults to `cli`.
        ///
        /// This is a **free-text, advisory** field — the caller
        /// picks the value. The trustworthy "this was AI-drafted"
        /// signal is the immutable `created_via` field stamped by
        /// `build_draft` itself, which the GUI install review uses
        /// to flag non-GUI agents.
        #[arg(long = "drafted-by", default_value = "cli")]
        drafted_by: String,
    },
    /// List every agent with its id, name, lifecycle, and trigger
    /// summary. Read-only.
    List,
    /// Print one agent's full spec. Accepts an agent id or name.
    /// Read-only.
    Show {
        /// Agent UUID or name.
        id: String,
    },
    /// Plumbing: invoked by an agent's helper shim after
    /// `claude -p` exits. Reads the redirected `stdout.log` from
    /// the per-run directory, parses the terminal `result` event,
    /// and writes `result.json` next to the logs.
    #[command(name = "_record-run")]
    RecordRun {
        // `--automation-id` alias keeps already-installed agent shims
        // (which pass `--automation-id`) working across the rename.
        #[arg(long = "agent-id", alias = "automation-id")]
        agent_id: String,
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        exit: i32,
        /// Unix seconds of start time. Optional — defaults to
        /// "now". Useful for environments where the shim can't
        /// compute timestamps reliably (e.g. Windows Task Scheduler
        /// contexts that don't inherit PATH).
        #[arg(long, default_value = "")]
        start: String,
        #[arg(long, default_value = "")]
        end: String,
        #[arg(long, default_value = "scheduled")]
        trigger: String,
        /// Absolute path to the run directory. Authoritative; the
        /// shim always passes this. When omitted (manual debug
        /// invocation), the default `~/.claudepot/automations/<id>/runs/<run-id>`
        /// path is used.
        #[arg(long)]
        run_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List all CC projects
    List,
    /// Show details for a project
    Show {
        /// Path to the project (resolved to absolute)
        path: String,
    },
    /// Move/rename a project and migrate CC state
    Move {
        #[command(flatten)]
        args: commands::project::MoveArgs,
    },
    /// Remove orphaned project directories
    Clean {
        /// Show what would be removed without deleting
        #[arg(long)]
        dry_run: bool,
        /// Proceed despite unresolved pending rename journals
        #[arg(long)]
        ignore_pending_journals: bool,
    },
    /// Move a single project to recoverable trash and strip its
    /// `~/.claude.json` + `history.jsonl` entries. Manual counterpart
    /// to `clean` for the "I accidentally ran `claude` here" case
    /// where the source cwd still exists. Restore via
    /// `claudepot project trash restore <id>`.
    Remove {
        /// Project path or sanitized slug (e.g. `/Users/joker` or
        /// `-Users-joker`).
        target: String,
        /// Show the disclosure without trashing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Inspect, restore, or permanently delete trashed projects
    Trash {
        #[command(subcommand)]
        action: ProjectTrashAction,
    },
    /// Repair or resolve pending / failed rename journals
    Repair {
        #[command(flatten)]
        args: commands::project::RepairArgs,
    },
    /// List project-scoped plugin bindings whose project directory no
    /// longer exists (e.g. after an external move). Fix with
    /// `project move <old> <new>`.
    PluginBindings,
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Read a bundle's manifest and print a summary without
    /// extracting.
    Inspect {
        /// Path to the bundle file.
        bundle: std::path::PathBuf,
        /// In-place upgrade older bundles to the current schema.
        #[arg(long)]
        upgrade_schema: bool,
    },
    /// Reverse the most recent import within the 24h undo window.
    Undo,
}

#[derive(Subcommand)]
enum ProjectTrashAction {
    /// List trashed projects (newest first)
    List,
    /// Restore a trashed project by its trash id
    Restore {
        /// Trash entry id (from `project trash list`)
        id: String,
    },
    /// Permanently delete trashed projects (irreversible)
    Empty {
        /// Only delete entries older than this many days (omit to
        /// empty everything that matches)
        #[arg(long)]
        older_than: Option<u64>,
    },
}

#[derive(Subcommand)]
enum AccountAction {
    /// List all registered accounts
    List,
    /// Register a new account
    Add {
        /// Import from current CC credentials
        #[arg(long, conflicts_with = "from_token")]
        from_current: bool,
        /// Bootstrap from a refresh token (reads from stdin if value is "-")
        #[arg(long, conflicts_with = "from_current")]
        from_token: Option<String>,
    },
    /// Remove a registered account
    Remove {
        /// Account email (prefix match)
        email: String,
    },
    /// Show detailed account information
    Inspect {
        /// Account email (prefix match)
        email: String,
    },
    /// Verify each per-account blob's identity against `/api/oauth/profile`.
    /// Detects misfiled slots where the stored blob authenticates as a
    /// different account than the label claims. Exit code: 0 all-ok,
    /// 2 drift, 3 rejected/network error.
    Verify {
        /// Account email (prefix match). Omit to verify every account.
        email: Option<String>,
    },
}

#[derive(Subcommand)]
enum CliAction {
    /// Show the active CLI account
    Status,
    /// Switch the active CLI account
    Use {
        /// Account email (prefix match)
        email: String,
        /// Skip automatic token refresh during switch
        #[arg(long)]
        no_refresh: bool,
        /// Proceed even if a Claude Code process is running (its token
        /// refresh may silently revert the swap)
        #[arg(long)]
        force: bool,
    },
    /// Clear CC credentials (log out)
    Clear,
    /// Launch a command with a specific account's token (Mode D)
    Run {
        /// Account email (prefix match)
        email: String,
        /// Print access token to stdout instead of launching
        #[arg(long)]
        print_token: bool,
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DesktopAction {
    /// Show the active Desktop account and running state
    Status,
    /// Switch the active Desktop account
    Use {
        /// Account email (prefix match)
        email: String,
        /// Don't relaunch Desktop after switching
        #[arg(long)]
        no_launch: bool,
    },
    /// Probe the live Desktop session identity.
    ///
    /// Phase 1 uses a fast, non-verifying org-UUID match against
    /// `~/Library/Application Support/Claude/config.json`. The result
    /// is labeled "org_uuid_candidate" so callers know not to trust it
    /// for mutation. Phase 2 will add a `--strict` path that decrypts
    /// `oauth:tokenCache` and verifies via `/profile`.
    Identity {
        /// Force the authoritative slow path (decrypt + /profile).
        /// Phase 1: returns an error since crypto isn't wired yet.
        #[arg(long)]
        strict: bool,
    },
    /// Reconcile `has_desktop_profile` flags with on-disk truth and
    /// clear orphan `state.active_desktop` pointers. Idempotent.
    Reconcile,
    /// Adopt the live Desktop session into an account's snapshot dir.
    /// Refuses unless the live identity (verified via /profile) matches
    /// the target account's stored email.
    Adopt {
        /// Account email (prefix match). Omit to adopt into whichever
        /// registered account's email the live /profile returns.
        email: Option<String>,
        /// Replace an existing snapshot for this account.
        #[arg(long)]
        overwrite: bool,
    },
    /// Sign Desktop out. Stashes the live session as a snapshot by
    /// default so the user can swap back in later.
    Clear {
        /// Delete the session items AND the snapshot — full wipe.
        /// Default behavior preserves the snapshot.
        #[arg(long)]
        no_keep_snapshot: bool,
    },
    /// Launch Claude Desktop.
    Launch,
    /// Gracefully quit Claude Desktop if it's running.
    Quit,
}

/// Shared context for all command handlers.
pub struct AppContext {
    pub store: AccountStore,
    pub usage_cache: UsageCache,
    pub json: bool,
    pub quiet: bool,
    pub yes: bool,
}

impl AppContext {
    fn info(&self, msg: &str) {
        if !self.quiet {
            eprintln!("{msg}");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Pin all tracing output to stderr regardless of subcommand. The
    // MCP memory-server subcommand uses stdout for JSON-RPC frames;
    // a single `tracing::info!` / `warn!` landing on the default
    // stdout writer would corrupt the protocol stream silently. This
    // is the single mandatory invariant for `claudepot mcp …`, but
    // it's safe everywhere else too — Claudepot's user-facing CLI
    // output uses `println!`, never `tracing`.
    if cli.verbose {
        // RUST_LOG wins when set — lets users pin noisy modules on the
        // fly (e.g. `RUST_LOG=claudepot_core::cli_backend=trace`).
        // Falls back to `claudepot=debug` when RUST_LOG is unset or
        // unparseable, preserving the prior default.
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("claudepot=debug"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();
    } else if matches!(cli.command, Commands::Mcp { .. }) {
        // MCP subcommand always installs a stderr-pinned subscriber
        // (even without --verbose) so deep-stack `tracing::warn!` from
        // `apply_schema` or the indexer can never land on stdout.
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();
    }

    let data_dir = paths::claudepot_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data dir: {}", data_dir.display()))?;

    // WAL housekeeping before any store opens its connection. Cleans
    // up leftover `*.db-wal` files that the previous claudepot exit
    // (clean or otherwise) didn't truncate — see
    // `crates/claudepot-core/src/db_housekeeping.rs` for context.
    // Concurrent CLI/GUI processes safely back off via busy_timeout.
    let reclaimed = claudepot_core::db_housekeeping::checkpoint_known_db_files(&data_dir);
    if reclaimed > 0 {
        tracing::debug!(bytes = reclaimed, "startup WAL checkpoint reclaimed bytes");
    }

    // One-time: migrate the legacy `~/.claude/claudepot/` repair tree
    // into `~/.claudepot/repair/`. Idempotent; safe on every invocation.
    // Non-fatal: log on failure but continue — falling back to reading
    // the old layout is fine since core still recognizes it.
    if let Err(e) = claudepot_core::migrations::migrate_repair_tree() {
        tracing::warn!("repair tree migration failed: {e}");
    }

    // Subcommands that don't touch the account store get dispatched
    // here, before AccountStore::open. None of these handlers consume
    // `ctx`, and opening accounts.db unconditionally on every CLI
    // invocation serializes parallel processes through SQLite's WAL
    // writer lock — the 5 s busy_timeout then trips and the surface
    // failure is a misleading "database is locked" that looks like a
    // test flake. Surfaced by the v0.1.36 CI run (26006793979)
    // where `cargo test`'s parallel `mcp memory-server` + `codex
    // index` subprocesses raced on the runner's accounts.db.
    // This early return is the ONLY dispatch site for Mcp/Codex —
    // the arms in the post-ctx match below are `unreachable!`. Wire
    // new Mcp/Codex verbs here. The global `--json` / `--yes` flags
    // are already parsed, so they pass straight into the handlers.
    if matches!(cli.command, Commands::Mcp { .. } | Commands::Codex { .. }) {
        return match cli.command {
            Commands::Mcp { action } => match action {
                McpAction::MemoryServer { db } => commands::mcp::run(db).await,
                McpAction::PrintSnippet => commands::mcp::print_snippet(),
                McpAction::InstallSnippet { out, print_include } => {
                    commands::mcp::install_snippet(out, print_include)
                }
            },
            Commands::Codex { action } => match action {
                CodexAction::Index { codex_home, db } => {
                    commands::codex::index(codex_home, db, cli.json).await
                }
                CodexAction::Rebuild { db } => commands::codex::rebuild(db, cli.json).await,
                CodexAction::Forget { db } => commands::codex::forget(db, cli.yes).await,
            },
            _ => unreachable!("matches! guard ensures Mcp or Codex"),
        };
    }

    let db_path = data_dir.join("accounts.db");
    let store = AccountStore::open(&db_path)
        .with_context(|| format!("failed to open account store: {}", db_path.display()))?;

    let ctx = AppContext {
        store,
        usage_cache: UsageCache::new(),
        json: cli.json,
        quiet: cli.quiet,
        yes: cli.yes,
    };

    match cli.command {
        Commands::Account { action } => match action {
            AccountAction::List => commands::account::list(&ctx).await?,
            AccountAction::Add {
                from_current,
                from_token,
            } => commands::account::add(&ctx, from_current, from_token).await?,
            AccountAction::Remove { email } => commands::account::remove(&ctx, &email).await?,
            AccountAction::Inspect { email } => commands::account::inspect(&ctx, &email).await?,
            AccountAction::Verify { email } => {
                commands::account::verify(&ctx, email.as_deref()).await?
            }
        },
        Commands::Cli { action } => match action {
            CliAction::Status => commands::cli_ops::status(&ctx).await?,
            CliAction::Use {
                email,
                no_refresh,
                force,
            } => commands::cli_ops::use_account(&ctx, &email, no_refresh, force).await?,
            CliAction::Clear => commands::cli_ops::clear(&ctx).await?,
            CliAction::Run {
                email,
                print_token,
                args,
            } => commands::cli_ops::run(&ctx, &email, print_token, &args).await?,
        },
        Commands::Desktop { action } => match action {
            DesktopAction::Status => commands::desktop_ops::status(&ctx).await?,
            DesktopAction::Use { email, no_launch } => {
                commands::desktop_ops::use_account(&ctx, &email, no_launch).await?
            }
            DesktopAction::Identity { strict } => {
                commands::desktop_ops::identity(&ctx, strict).await?
            }
            DesktopAction::Reconcile => commands::desktop_ops::reconcile(&ctx).await?,
            DesktopAction::Adopt { email, overwrite } => {
                commands::desktop_ops::adopt(&ctx, email.as_deref(), overwrite).await?
            }
            DesktopAction::Clear { no_keep_snapshot } => {
                commands::desktop_ops::clear(&ctx, !no_keep_snapshot).await?
            }
            DesktopAction::Launch => commands::desktop_ops::launch(&ctx).await?,
            DesktopAction::Quit => commands::desktop_ops::quit(&ctx).await?,
        },
        Commands::Project { action } => match action {
            ProjectAction::List => commands::project::list(&ctx)?,
            ProjectAction::Show { path } => commands::project::show(&ctx, &path)?,
            ProjectAction::Move { args } => commands::project::move_project(&ctx, args)?,
            ProjectAction::Clean {
                dry_run,
                ignore_pending_journals,
            } => commands::project::clean(&ctx, dry_run, ignore_pending_journals)?,
            ProjectAction::Remove { target, dry_run } => {
                commands::project::remove(&ctx, &target, dry_run)?
            }
            ProjectAction::Trash { action } => match action {
                ProjectTrashAction::List => commands::project::trash_list(&ctx)?,
                ProjectTrashAction::Restore { id } => commands::project::trash_restore(&ctx, &id)?,
                ProjectTrashAction::Empty { older_than } => {
                    commands::project::trash_empty(&ctx, older_than)?
                }
            },
            ProjectAction::Repair { args } => commands::project::repair(&ctx, args)?,
            ProjectAction::PluginBindings => commands::project::plugin_bindings(&ctx)?,
        },
        Commands::Agent { action } => match *action {
            AgentAction::Draft {
                from_json,
                name,
                cwd,
                prompt,
                display_name,
                description,
                model,
                permission_mode,
                allowed_tools,
                disallowed_tools,
                cron,
                timezone,
                run_as,
                task_budget,
                attach_memory,
                drafted_by,
            } => commands::agent::draft_cmd(
                &ctx,
                commands::agent::DraftArgs {
                    from_json,
                    name,
                    cwd,
                    prompt,
                    display_name,
                    description,
                    model,
                    permission_mode,
                    allowed_tools,
                    disallowed_tools,
                    cron,
                    timezone,
                    run_as,
                    task_budget,
                    attach_memory,
                    drafted_by,
                },
            )?,
            AgentAction::List => commands::agent::list_cmd(&ctx)?,
            AgentAction::Show { id } => commands::agent::show_cmd(&ctx, &id)?,
            AgentAction::RecordRun {
                agent_id,
                run_id,
                exit,
                start,
                end,
                trigger,
                run_dir,
            } => commands::agent::record_run_cmd(
                &agent_id,
                &run_id,
                exit,
                &start,
                &end,
                &trigger,
                run_dir.as_deref(),
            )?,
        },
        Commands::Export { args } => commands::project_migrate::export(&ctx, args)?,
        Commands::Import { args } => commands::project_migrate::import(&ctx, args)?,
        Commands::Migrate { action } => match action {
            MigrateAction::Inspect {
                bundle,
                upgrade_schema,
            } => commands::project_migrate::inspect(&ctx, bundle, upgrade_schema)?,
            MigrateAction::Undo => commands::project_migrate::undo(&ctx)?,
        },
        Commands::Doctor => commands::doctor::run(&ctx).await?,
        Commands::Logs { open, tail } => commands::logs::run(&ctx, open, tail).await?,
        Commands::Status => commands::status::run(&ctx).await?,
        Commands::Usage { action } => match action {
            UsageAction::Report { window } => commands::usage::report(&ctx, &window).await?,
        },
        Commands::Update { action } => match action {
            UpdateAction::Check => commands::update::check::run(&ctx).await?,
            UpdateAction::Cli => commands::update::cli::run(&ctx).await?,
            UpdateAction::Desktop => commands::update::desktop::run(&ctx).await?,
            UpdateAction::Config { args } => commands::update::config::run(&ctx, args).await?,
        },
        // Mcp/Codex never reach this match — they are dispatched (and
        // returned from) before AccountStore::open so they don't
        // serialize on accounts.db. Keeping the arms `unreachable!`
        // instead of duplicating the dispatch means an edit to only
        // one copy can't silently diverge.
        Commands::Mcp { .. } | Commands::Codex { .. } => {
            unreachable!("dispatched before AccountStore::open — see the early return above")
        }
        Commands::Activity { action } => match action {
            ActivityAction::Recent { args } => commands::activity::recent(&ctx, args)?,
            ActivityAction::Reindex => commands::activity::reindex(&ctx)?,
        },
        Commands::Memory { action } => match action {
            MemoryAction::List { project } => {
                commands::memory::list(&ctx, project.as_deref()).await?
            }
            MemoryAction::View { file, project } => {
                commands::memory::view(&ctx, &file, project.as_deref()).await?
            }
            MemoryAction::Log {
                project,
                file,
                limit,
                show_diff,
            } => {
                commands::memory::log(&ctx, project.as_deref(), file.as_deref(), limit, show_diff)
                    .await?
            }
        },
        Commands::Settings { action } => match action {
            SettingsAction::AutoMemory { action } => match action {
                AutoMemoryAction::Status { project } => {
                    commands::settings::auto_memory_status(&ctx, project.as_deref()).await?
                }
                AutoMemoryAction::Enable {
                    project,
                    project_scope,
                } => {
                    commands::settings::auto_memory_enable(
                        &ctx,
                        project.as_deref(),
                        project_scope || project.is_some(),
                    )
                    .await?
                }
                AutoMemoryAction::Disable {
                    project,
                    project_scope,
                } => {
                    commands::settings::auto_memory_disable(
                        &ctx,
                        project.as_deref(),
                        project_scope || project.is_some(),
                    )
                    .await?
                }
                AutoMemoryAction::Clear {
                    project,
                    project_scope,
                } => {
                    commands::settings::auto_memory_clear(
                        &ctx,
                        project.as_deref(),
                        project_scope || project.is_some(),
                    )
                    .await?
                }
            },
        },
        Commands::Session { action } => match action {
            SessionAction::ListOrphans => commands::session::list_orphans(&ctx)?,
            SessionAction::Move {
                session_id,
                from,
                to,
                force_live,
                force_conflict,
                cleanup_source,
            } => commands::session::move_cmd(
                &ctx,
                &session_id,
                &from,
                &to,
                force_live,
                force_conflict,
                cleanup_source,
            )?,
            SessionAction::AdoptOrphan { slug, target } => {
                commands::session::adopt_orphan_cmd(&ctx, &slug, &target)?
            }
            SessionAction::RebuildIndex => commands::session::rebuild_index_cmd(&ctx)?,
            SessionAction::BackfillExchanges { claude_config } => {
                commands::session::backfill_exchanges_cmd(&ctx, claude_config).await?
            }
            SessionAction::View { target, show } => {
                commands::session::view_cmd(&ctx, &target, &show)?
            }
            SessionAction::Export { args } => commands::session::export_cmd(&ctx, args).await?,
            SessionAction::Search { query, limit } => {
                commands::session::search_cmd(&ctx, &query, limit)?
            }
            SessionAction::Worktrees => commands::session::worktrees_cmd(&ctx)?,
            SessionAction::Prune { args } => commands::session::prune_cmd(&ctx, args)?,
            SessionAction::Slim { args } => commands::session::slim_cmd(&ctx, args)?,
            SessionAction::Trash { action } => match action {
                TrashAction::List { older_than } => {
                    commands::session::trash_list_cmd(&ctx, older_than.as_deref())?
                }
                TrashAction::Restore { id, to } => {
                    commands::session::trash_restore_cmd(&ctx, &id, to.as_deref())?
                }
                TrashAction::Empty { older_than } => {
                    commands::session::trash_empty_cmd(&ctx, older_than.as_deref())?
                }
            },
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_assert_no_arg_conflicts() {
        use clap::CommandFactory;
        // clap's self-check. Catches duplicate arg ids — e.g. a
        // subcommand-local `--yes`/`--json` shadowing the global
        // flag, which only asserts in debug builds at parse time.
        Cli::command().debug_assert();
    }

    #[test]
    fn test_codex_index_global_json_propagates_any_position() {
        let before = Cli::try_parse_from(["claudepot", "--json", "codex", "index"]).unwrap();
        assert!(before.json);
        let after = Cli::try_parse_from(["claudepot", "codex", "index", "--json"]).unwrap();
        assert!(after.json);
        // The short form must work too — the old local flag broke it.
        let short = Cli::try_parse_from(["claudepot", "codex", "index", "-j"]).unwrap();
        assert!(short.json);
    }

    #[test]
    fn test_codex_forget_global_yes_propagates_any_position() {
        // The destructive-confirm flag must not be position-dependent:
        // the old subcommand-local `--yes` shadowed the global one.
        let before = Cli::try_parse_from(["claudepot", "-y", "codex", "forget"]).unwrap();
        assert!(before.yes);
        let after = Cli::try_parse_from(["claudepot", "codex", "forget", "--yes"]).unwrap();
        assert!(after.yes);
    }

    #[test]
    fn test_session_backfill_global_json_propagates() {
        let cli =
            Cli::try_parse_from(["claudepot", "--json", "session", "backfill-exchanges"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn test_logs_open_rejects_tail_combination() {
        // `--tail` never returns, so `--open` would be silently
        // dropped; the pair is a clap-level conflict instead.
        assert!(Cli::try_parse_from(["claudepot", "logs", "--open", "--tail"]).is_err());
        assert!(Cli::try_parse_from(["claudepot", "logs", "--open"]).is_ok());
        assert!(Cli::try_parse_from(["claudepot", "logs", "--tail"]).is_ok());
    }

    #[test]
    fn test_session_slim_target_conflicts_with_all_after_flatten() {
        // The flatten refactor must preserve the clap-level conflict.
        assert!(Cli::try_parse_from(["claudepot", "session", "slim", "abc", "--all"]).is_err());
        assert!(Cli::try_parse_from(["claudepot", "session", "slim", "--all"]).is_ok());
    }

    #[test]
    fn test_project_move_merge_conflicts_with_overwrite_after_flatten() {
        // The flatten refactor must preserve the clap-level conflict.
        assert!(Cli::try_parse_from([
            "claudepot",
            "project",
            "move",
            "/a",
            "/b",
            "--merge",
            "--overwrite"
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["claudepot", "project", "move", "/a", "/b", "--merge"]).is_ok()
        );
    }

    #[test]
    fn test_project_repair_flag_relationships_after_flatten() {
        // --rollback conflicts with --resume; --older-than requires --gc.
        assert!(
            Cli::try_parse_from(["claudepot", "project", "repair", "--resume", "--rollback"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["claudepot", "project", "repair", "--older-than", "30"]).is_err()
        );
        assert!(Cli::try_parse_from([
            "claudepot",
            "project",
            "repair",
            "--gc",
            "--older-than",
            "30"
        ])
        .is_ok());
    }

    #[test]
    fn test_export_flattened_flags_parse() {
        let cli = Cli::try_parse_from([
            "claudepot",
            "export",
            "myproj",
            "--include-global",
            "--out",
            "/tmp/x.claudepot.tar.zst",
        ])
        .unwrap();
        match cli.command {
            Commands::Export { args } => {
                assert_eq!(args.project_prefixes, vec!["myproj".to_string()]);
                assert!(args.include_global);
                assert_eq!(
                    args.out,
                    Some(std::path::PathBuf::from("/tmp/x.claudepot.tar.zst"))
                );
            }
            _ => panic!("expected Commands::Export"),
        }
    }
}
