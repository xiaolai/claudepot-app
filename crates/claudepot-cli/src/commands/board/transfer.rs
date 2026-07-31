//! Transfer verbs: `export`, `import`.
//!
//! Plan §12 forbids automatic deletion, which makes these v1 rather
//! than later polish — user data with no way out is a trap.

use super::*;
use crate::output::print_json;
use crate::AppContext;
use claudepot_core::board::{export_csv_dir, export_json, import_json};

/// `board export <board-id> --format json|csv --out <path|->`
pub fn export_cmd(ctx: &AppContext, board_id: &str, format: &str, out: &str) -> Result<()> {
    let store = store()?;

    match format {
        "json" => {
            if out == "-" {
                // Streams straight to stdout so a large board never
                // needs a temp file or a full in-memory copy.
                let stdout = std::io::stdout();
                let mut handle = std::io::BufWriter::new(stdout.lock());
                export_json(&store, board_id, &mut handle)?;
                use std::io::Write;
                handle.flush()?;
            } else {
                let file = std::fs::File::create(out).with_context(|| format!("creating {out}"))?;
                let mut w = std::io::BufWriter::new(file);
                export_json(&store, board_id, &mut w)?;
                if !ctx.quiet {
                    eprintln!("wrote {out}");
                }
            }
        }
        "csv" => {
            if out == "-" {
                anyhow::bail!(
                    "csv export writes one file per series, so --out must be a directory"
                );
            }
            let written = export_csv_dir(&store, board_id, std::path::Path::new(out))?;
            if ctx.json {
                let paths: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
                return print_json(&paths);
            }
            for p in &written {
                println!("{}", p.display());
            }
        }
        other => anyhow::bail!("unknown format `{other}` (json or csv)"),
    }
    Ok(())
}

/// `board import --from <file|->`
///
/// Always creates a new board. The envelope's original id is recorded
/// as `source_board_id` rather than reused — reusing an id needs a
/// written collision policy, and there isn't one.
pub fn import_cmd(ctx: &AppContext, from: &str) -> Result<()> {
    let raw = read_input(from)?;
    let store = store()?;
    let board_id = import_json(&store, &raw).context("importing board envelope")?;

    if ctx.json {
        return print_json(&serde_json::json!({ "board_id": board_id }));
    }
    println!("{board_id}");
    if !ctx.quiet {
        // Say it out loud: an envelope's provenance is whatever the
        // file claimed, and importing does not upgrade it.
        eprintln!("imported as a new board; reported provenance preserved and still unverified");
    }
    Ok(())
}
