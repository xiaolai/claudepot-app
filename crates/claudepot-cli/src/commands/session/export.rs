//! `export` verb — write a session transcript to file (markdown/html/json).
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::inspect::resolve_detail;
use super::*;

/// Flag bundle for `claudepot session export`, flattened into the
/// `SessionAction::Export` variant in `main.rs`.
#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    /// Session UUID or absolute `.jsonl` path.
    pub target: String,
    /// Output format.
    #[arg(long, default_value = "md", value_parser = ["md", "markdown", "markdown-slim", "json", "html"])]
    pub format: String,
    /// Destination. `file` requires --output; clipboard copies;
    /// gist uploads via GITHUB_TOKEN env or keychain.
    #[arg(long, default_value = "file", value_parser = ["file", "clipboard", "gist"])]
    pub to: String,
    /// Output file path (for --to file).
    #[arg(long)]
    pub output: Option<String>,
    /// Make the gist public (for --to gist). Default is secret.
    #[arg(long)]
    pub public: bool,
    /// Redact absolute paths: off | relative | hash.
    #[arg(long, default_value = "off", value_parser = ["off", "relative", "hash"])]
    pub redact_paths: String,
    /// Mask email-like strings with <email-redacted>.
    #[arg(long)]
    pub redact_emails: bool,
    /// Drop lines that look like FOO=bar env assignments.
    #[arg(long)]
    pub redact_env: bool,
    /// Repeatable: extra literal substrings to redact.
    #[arg(long)]
    pub redact_regex: Vec<String>,
    /// Strip the copy-buttons script from HTML output.
    #[arg(long)]
    pub html_no_js: bool,
}

/// `claudepot session export <target> --format fmt --to dest [flags]`
///
/// Pure dispatcher: format → policy → body → `core::session_export_delivery::deliver`.
/// All file/clipboard/gist mechanics live in `claudepot-core`; the CLI
/// only supplies a [`SubprocessClipboard`] for the `clipboard` arm.
pub async fn export_cmd(ctx: &AppContext, args: ExportArgs) -> Result<()> {
    use claudepot_core::session_export_delivery::{
        default_export_filename, deliver, extension_for, DeliveryReceipt, ExportDestination,
    };
    let ExportArgs {
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
    } = args;
    let detail = resolve_detail(&target)?;
    let fmt = match format.as_str() {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "markdown-slim" => claudepot_core::session_export::ExportFormat::MarkdownSlim,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        "html" => claudepot_core::session_export::ExportFormat::Html { no_js: html_no_js },
        other => bail!("unknown format: {other}"),
    };
    let policy = build_policy(&redact_paths, redact_emails, redact_env, redact_regex)?;
    let body = claudepot_core::session_export::export_with(&detail, fmt.clone(), &policy);
    let dest = match to.as_str() {
        "file" => {
            let path = output.ok_or_else(|| anyhow::anyhow!("--output required for --to file"))?;
            ExportDestination::File {
                path: PathBuf::from(path),
            }
        }
        "clipboard" => ExportDestination::Clipboard,
        "gist" => ExportDestination::Gist {
            filename: default_export_filename(&detail.row.session_id, extension_for(&fmt)),
            description: format!("Claudepot session export: {}", detail.row.session_id),
            public,
        },
        other => bail!("unknown destination: {other}"),
    };
    let clipboard = crate::clipboard::SubprocessClipboard;
    let receipt = deliver(
        &body,
        dest,
        Some(&clipboard),
        &claudepot_core::project_progress::NoopSink,
    )
    .await?;
    match receipt {
        DeliveryReceipt::File { path, bytes } => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "destination": "file",
                        "path": path,
                        "bytes": bytes,
                    }))?
                );
            } else {
                // Result, not progress — stdout per `rules/commands.md`.
                println!("Wrote {bytes} bytes to {}", path.display());
            }
        }
        DeliveryReceipt::Clipboard { bytes } => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "destination": "clipboard",
                        "bytes": bytes,
                    }))?
                );
            } else {
                // Result, not progress — stdout per `rules/commands.md`.
                println!("Copied {bytes} bytes to clipboard");
            }
        }
        DeliveryReceipt::Gist { result, .. } => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "destination": "gist",
                        "url": result.url,
                        "id": result.id,
                    }))?
                );
            } else {
                eprintln!("Uploaded to {}", result.url);
                println!("{}", result.url);
            }
        }
    }
    Ok(())
}

fn build_policy(
    redact_paths: &str,
    redact_emails: bool,
    redact_env: bool,
    redact_regex: Vec<String>,
) -> Result<claudepot_core::redaction::RedactionPolicy> {
    use claudepot_core::redaction::{PathStrategy, RedactionPolicy};
    let paths = match redact_paths {
        "off" => PathStrategy::Off,
        "relative" => PathStrategy::Relative {
            root: dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")),
        },
        "hash" => PathStrategy::Hash,
        other => bail!("unknown redact-paths strategy: {other}"),
    };
    Ok(RedactionPolicy {
        anthropic_keys: true,
        paths,
        emails: redact_emails,
        env_assignments: redact_env,
        custom_regex: redact_regex,
    })
}
