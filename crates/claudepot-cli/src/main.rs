use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "claudepot", about = "Multi-account Claude Code/Desktop switcher")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage registered accounts
    Accounts {
        #[command(subcommand)]
        action: AccountsAction,
    },
    /// CLI account switching (Mode A)
    Cli {
        #[command(subcommand)]
        action: CliAction,
    },
    /// Desktop account switching
    Desktop {
        #[command(subcommand)]
        action: DesktopAction,
    },
    /// Show usage for one or all accounts
    Usage {
        /// Account name (omit for all)
        name: Option<String>,
    },
    /// Launch claude with a specific account's token (Mode D)
    Run {
        /// Account name
        #[arg(long)]
        account: String,
        /// Arguments to pass to claude
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum AccountsAction {
    /// List all registered accounts
    List,
    /// Add a new account (opens browser for OAuth)
    Add,
    /// Remove an account
    Remove { name: String },
    /// Show account details
    Info { name: String },
}

#[derive(Subcommand)]
enum CliAction {
    /// Show the currently active CLI account
    Status,
    /// Switch the active CLI account
    Switch { name: String },
}

#[derive(Subcommand)]
enum DesktopAction {
    /// Show the currently active Desktop account
    Status,
    /// Switch the active Desktop account
    Switch { name: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Accounts { action } => match action {
            AccountsAction::List => {
                println!("(stub) claudepot accounts list");
            }
            AccountsAction::Add => {
                println!("(stub) claudepot accounts add");
            }
            AccountsAction::Remove { name } => {
                println!("(stub) claudepot accounts remove {name}");
            }
            AccountsAction::Info { name } => {
                println!("(stub) claudepot accounts info {name}");
            }
        },
        Commands::Cli { action } => match action {
            CliAction::Status => {
                println!("(stub) claudepot cli status");
            }
            CliAction::Switch { name } => {
                println!("(stub) claudepot cli switch {name}");
            }
        },
        Commands::Desktop { action } => match action {
            DesktopAction::Status => {
                println!("(stub) claudepot desktop status");
            }
            DesktopAction::Switch { name } => {
                println!("(stub) claudepot desktop switch {name}");
            }
        },
        Commands::Usage { name } => {
            println!("(stub) claudepot usage {}", name.as_deref().unwrap_or("(all)"));
        }
        Commands::Run { account, args } => {
            println!("(stub) claudepot run --account {account} -- {}", args.join(" "));
        }
    }

    Ok(())
}
