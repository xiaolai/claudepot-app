//! `export` verb — write a session transcript to file (markdown/html/json).
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;
use super::inspect::resolve_detail;

/// `claudepot session export <target> --format fmt --to dest [flags]`
///
/// Pure dispatcher: format → policy → body → `core::session_export_delivery::deliver`.
/// All file/clipboard/gist mechanics live in `claudepot-core`; the CLI
/// only supplies a [`SubprocessClipboard`] for the `clipboard` arm.
#[allow(clippy::too_many_arguments)]
pub async fn export_cmd(
    ctx: &AppContext,
    target: &str,
    format: &str,
    to: &str,
    output: Option<&str>,
    public: bool,
    redact_paths: &str,
    redact_emails: bool,
    redact_env: bool,
    redact_regex: Vec<String>,
    html_no_js: bool,
) -> Result<()> {
    use claudepot_core::session_export_delivery::{
        deliver, default_export_filename, extension_for, DeliveryReceipt, ExportDestination,
    };
    let _ = ctx;
    let detail = resolve_detail(target)?;
    let fmt = match format {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "markdown-slim" => claudepot_core::session_export::ExportFormat::MarkdownSlim,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        "html" => claudepot_core::session_export::ExportFormat::Html {
            no_js: html_no_js,
        },
        other => bail!("unknown format: {other}"),
    };
    let policy = build_policy(redact_paths, redact_emails, redact_env, redact_regex)?;
    let body = claudepot_core::session_export::export_with(&detail, fmt.clone(), &policy);
    let dest = match to {
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
            eprintln!("Wrote {bytes} bytes to {}", path.display());
        }
        DeliveryReceipt::Clipboard { bytes } => {
            eprintln!("Copied {bytes} bytes to clipboard");
        }
        DeliveryReceipt::Gist { result, .. } => {
            eprintln!("Uploaded to {}", result.url);
            println!("{}", result.url);
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
