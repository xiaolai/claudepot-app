use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use claudepot_core::account::AccountStore;
use claudepot_core::paths;

mod commands;
mod output;

#[derive(Parser)]
#[command(name = "claudepot", about = "Multi-account Claude Code / Desktop switcher")]
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
    /// Health check and diagnostics
    Doctor,
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
    },
    /// Remove orphaned project directories
    Clean {
        /// Show what would be removed without deleting
        #[arg(long)]
        dry_run: bool,
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
}

/// Shared context for all command handlers.
pub struct AppContext {
    pub store: AccountStore,
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
        tracing_subscriber::fmt()
            .with_env_filter("claudepot=debug")
            .init();
    }

    let data_dir = paths::claudepot_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data dir: {}", data_dir.display()))?;

    let db_path = data_dir.join("accounts.db");
    let store = AccountStore::open(&db_path)
        .with_context(|| format!("failed to open account store: {}", db_path.display()))?;

    let ctx = AppContext {
        store,
        json: cli.json,
        quiet: cli.quiet,
        yes: cli.yes,
    };

    match cli.command {
        Commands::Account { action } => match action {
            AccountAction::List => commands::account::list(&ctx)?,
            AccountAction::Add { from_current, from_token } => {
                commands::account::add(&ctx, from_current, from_token).await?
            }
            AccountAction::Remove { email } => commands::account::remove(&ctx, &email)?,
            AccountAction::Inspect { email } => commands::account::inspect(&ctx, &email).await?,
        },
        Commands::Cli { action } => match action {
            CliAction::Status => commands::cli_ops::status(&ctx)?,
            CliAction::Use { email, no_refresh } => {
                commands::cli_ops::use_account(&ctx, &email, no_refresh).await?
            }
            CliAction::Clear => commands::cli_ops::clear(&ctx).await?,
            CliAction::Run { email, print_token, args } => {
                commands::cli_ops::run(&ctx, &email, print_token, &args).await?
            }
        },
        Commands::Desktop { action } => match action {
            DesktopAction::Status => commands::desktop_ops::status(&ctx).await?,
            DesktopAction::Use { email, no_launch } => {
                commands::desktop_ops::use_account(&ctx, &email, no_launch).await?
            }
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
            } => {
                commands::project::move_project(
                    &ctx, &old_path, &new_path, no_move, merge, overwrite, force, dry_run,
                )?
            }
            ProjectAction::Clean { dry_run } => commands::project::clean(&ctx, dry_run)?,
        },
        Commands::Doctor => commands::doctor::run(&ctx).await?,
    }

    Ok(())
}
