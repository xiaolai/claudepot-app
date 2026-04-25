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
    /// Health check and diagnostics
    Doctor,
    /// Ground-truth authentication status.
    ///
    /// Reads CC's shared credential slot, calls `/api/oauth/profile`,
    /// compares the verified email to Claudepot's `active_cli` pointer.
    /// Prints MATCH / DRIFT / NOT SIGNED IN. Exit code: 0 match,
    /// 2 drift, 3 couldn't check.
    Status,
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
        /// Session UUID or absolute `.jsonl` path.
        target: String,
        /// Output format.
        #[arg(long, default_value = "md", value_parser = ["md", "markdown", "markdown-slim", "json", "html"])]
        format: String,
        /// Destination. `file` requires --output; clipboard copies;
        /// gist uploads via GITHUB_TOKEN env or keychain.
        #[arg(long, default_value = "file", value_parser = ["file", "clipboard", "gist"])]
        to: String,
        /// Output file path (for --to file).
        #[arg(long)]
        output: Option<String>,
        /// Make the gist public (for --to gist). Default is secret.
        #[arg(long)]
        public: bool,
        /// Redact absolute paths: off | relative | hash.
        #[arg(long, default_value = "off", value_parser = ["off", "relative", "hash"])]
        redact_paths: String,
        /// Mask email-like strings with <email-redacted>.
        #[arg(long)]
        redact_emails: bool,
        /// Drop lines that look like FOO=bar env assignments.
        #[arg(long)]
        redact_env: bool,
        /// Repeatable: extra literal substrings to redact.
        #[arg(long)]
        redact_regex: Vec<String>,
        /// Strip the copy-buttons script from HTML output.
        #[arg(long)]
        html_no_js: bool,
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
        /// Match sessions whose last activity is older than the given
        /// duration. Accepts `7d`, `24h`, `90m`, `3600s`.
        #[arg(long)]
        older_than: Option<String>,
        /// Match sessions whose size is at least the given value.
        /// Accepts `10MB`, `500KB`, `1024`.
        #[arg(long)]
        larger_than: Option<String>,
        /// Repeatable: narrow to sessions whose cwd equals one of these.
        #[arg(long)]
        project: Vec<String>,
        /// Only include sessions that recorded an error.
        #[arg(long)]
        has_error: bool,
        /// Only include sidechain (subagent) sessions.
        #[arg(long)]
        sidechain: bool,
        /// Actually move files into the trash. Without this flag,
        /// prune only prints the plan.
        #[arg(long)]
        execute: bool,
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
        /// Session UUID or absolute `.jsonl` path. Omit with `--all`.
        #[arg(conflicts_with = "all")]
        target: Option<String>,
        /// Run against every session matching the filter flags below.
        /// Mutually exclusive with `<target>`.
        #[arg(long)]
        all: bool,
        /// Filter: only sessions whose last_ts is older than this (e.g. `7d`, `30d`).
        /// Requires `--all`.
        #[arg(long)]
        older_than: Option<String>,
        /// Filter: only sessions at least this size (`1MB`, `500KB`).
        /// Requires `--all`.
        #[arg(long)]
        larger_than: Option<String>,
        /// Filter: repeatable project path filter. Requires `--all`.
        #[arg(long)]
        project: Vec<String>,
        /// Drop tool_result payloads larger than this. Accepts
        /// `1MB`, `500KB`, `1024`. Default: 1MiB.
        #[arg(long)]
        drop_tool_results_over: Option<String>,
        /// Repeatable: tool names whose results to preserve regardless.
        #[arg(long)]
        exclude_tool: Vec<String>,
        /// Replace base64 image blocks with `[image]` text stubs.
        /// Saves ~2000 tokens/image on `claude --resume` of this
        /// session.
        #[arg(long)]
        strip_images: bool,
        /// Replace document (PDF etc.) blocks with `[document]` text
        /// stubs. Same ~2000-token-per-block accounting as images.
        #[arg(long)]
        strip_documents: bool,
        /// Actually rewrite the file. Without this, slim only plans.
        #[arg(long)]
        execute: bool,
    },
}

#[derive(Subcommand)]
enum ActivityAction {
    /// Show recent activity cards (anomalies + milestones), newest first.
    Recent {
        /// Window: `30m`, `2h`, `7d`. Omit for all-time.
        #[arg(long)]
        since: Option<String>,
        /// Filter by kind. Repeat for multiple kinds. Values:
        /// hook, hook-slow, hook-info, agent, agent-stranded,
        /// tool-error, command, milestone.
        #[arg(long, value_name = "KIND")]
        kind: Vec<String>,
        /// Minimum severity: info, notice, warn, error.
        #[arg(long)]
        severity: Option<String>,
        /// Filter to cards from this project (matches by cwd prefix).
        #[arg(long)]
        project: Option<String>,
        /// Filter to cards attributed to this plugin (`<name>` or
        /// `<name>@<owner>`).
        #[arg(long)]
        plugin: Option<String>,
        /// Maximum rows to print. Defaults to 200, capped at 10000.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Walk every JSONL under `~/.claude/projects/` and rebuild the
    /// activity index. Idempotent — re-running adds zero rows when
    /// the source hasn't changed.
    Reindex,
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
        /// Current project path
        old_path: String,
        /// New project path
        new_path: String,
        /// Only update CC state, don't move the actual directory
        #[arg(long)]
        no_move: bool,
        /// Merge CC data if target already has sessions
        #[arg(long, conflicts_with = "overwrite")]
        merge: bool,
        /// Overwrite CC data at target
        #[arg(long, conflicts_with = "merge")]
        overwrite: bool,
        /// Proceed even if Claude is running in the directory
        #[arg(long)]
        force: bool,
        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
        /// Proceed despite unresolved pending rename journals (last-resort)
        #[arg(long)]
        ignore_pending_journals: bool,
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
    /// Repair or resolve pending / failed rename journals
    Repair {
        /// Finish remaining phases for a journal (id optional, use --all to target every one)
        #[arg(long)]
        resume: bool,
        /// Reverse completed phases and restore snapshots
        #[arg(long, conflicts_with = "resume")]
        rollback: bool,
        /// Mark a journal as abandoned (keeps audit trail, suppresses nags)
        #[arg(long, conflicts_with_all = ["resume", "rollback"])]
        abandon: bool,
        /// Force-release a lock file whose staleness detection refuses auto-break
        #[arg(long)]
        break_lock: Option<String>,
        /// Clean up abandoned journals and expired snapshots
        #[arg(long)]
        gc: bool,
        /// For --gc: how many days old before cleanup (default 90)
        #[arg(long, requires = "gc")]
        older_than: Option<u64>,
        /// Target journal id (filename without extension). If absent,
        /// --resume/--rollback/--abandon require --all.
        #[arg(long)]
        id: Option<String>,
        /// Apply to all matching journals
        #[arg(long)]
        all: bool,
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

    if cli.verbose {
        // RUST_LOG wins when set — lets users pin noisy modules on the
        // fly (e.g. `RUST_LOG=claudepot_core::cli_backend=trace`).
        // Falls back to `claudepot=debug` when RUST_LOG is unset or
        // unparseable, preserving the prior default.
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("claudepot=debug"));
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let data_dir = paths::claudepot_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data dir: {}", data_dir.display()))?;

    // One-time: migrate the legacy `~/.claude/claudepot/` repair tree
    // into `~/.claudepot/repair/`. Idempotent; safe on every invocation.
    // Non-fatal: log on failure but continue — falling back to reading
    // the old layout is fine since core still recognizes it.
    if let Err(e) = claudepot_core::migrations::migrate_repair_tree() {
        tracing::warn!("repair tree migration failed: {e}");
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
            CliAction::Use { email, no_refresh, force } => {
                commands::cli_ops::use_account(&ctx, &email, no_refresh, force).await?
            }
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
            ProjectAction::Move {
                old_path,
                new_path,
                no_move,
                merge,
                overwrite,
                force,
                dry_run,
                ignore_pending_journals,
            } => commands::project::move_project(
                &ctx,
                &old_path,
                &new_path,
                no_move,
                merge,
                overwrite,
                force,
                dry_run,
                ignore_pending_journals,
            )?,
            ProjectAction::Clean {
                dry_run,
                ignore_pending_journals,
            } => commands::project::clean(&ctx, dry_run, ignore_pending_journals)?,
            ProjectAction::Repair {
                resume,
                rollback,
                abandon,
                break_lock,
                gc,
                older_than,
                id,
                all,
            } => commands::project::repair(
                &ctx,
                resume,
                rollback,
                abandon,
                break_lock.as_deref(),
                gc,
                older_than,
                id.as_deref(),
                all,
            )?,
        },
        Commands::Doctor => commands::doctor::run(&ctx).await?,
        Commands::Status => commands::status::run(&ctx).await?,
        Commands::Activity { action } => match action {
            ActivityAction::Recent {
                since,
                kind,
                severity,
                project,
                plugin,
                limit,
            } => commands::activity::recent(
                &ctx,
                since.as_deref(),
                &kind,
                severity.as_deref(),
                project.as_deref(),
                plugin.as_deref(),
                limit,
            )?,
            ActivityAction::Reindex => commands::activity::reindex(&ctx)?,
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
            SessionAction::View { target, show } => {
                commands::session::view_cmd(&ctx, &target, &show)?
            }
            SessionAction::Export {
                target,
                format,
                to,
                output,
                public,
                redact_paths,
                redact_emails,
                redact_env,
                redact_regex,
                html_no_js,
            } => commands::session::export_cmd(
                &ctx,
                &target,
                &format,
                &to,
                output.as_deref(),
                public,
                &redact_paths,
                redact_emails,
                redact_env,
                redact_regex,
                html_no_js,
            )
            .await?,
            SessionAction::Search { query, limit } => {
                commands::session::search_cmd(&ctx, &query, limit)?
            }
            SessionAction::Worktrees => commands::session::worktrees_cmd(&ctx)?,
            SessionAction::Prune {
                older_than,
                larger_than,
                project,
                has_error,
                sidechain,
                execute,
            } => commands::session::prune_cmd(
                &ctx,
                older_than.as_deref(),
                larger_than.as_deref(),
                project,
                has_error,
                sidechain,
                execute,
            )?,
            SessionAction::Slim {
                target,
                all,
                older_than,
                larger_than,
                project,
                drop_tool_results_over,
                exclude_tool,
                strip_images,
                strip_documents,
                execute,
            } => commands::session::slim_cmd(
                &ctx,
                target.as_deref(),
                all,
                older_than.as_deref(),
                larger_than.as_deref(),
                project,
                drop_tool_results_over.as_deref(),
                exclude_tool,
                strip_images,
                strip_documents,
                execute,
            )?,
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
