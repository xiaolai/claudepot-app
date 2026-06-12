//! `view` verb — print a memory file's contents.
//!
//! Sub-module of `commands/memory.rs`; see that file's header for
//! the per-verb layout rationale and the shared resolution helpers.

use super::*;

/// `claudepot memory view <FILE> [--project <PATH>]` — print contents.
pub async fn view(ctx: &AppContext, file: &str, project: Option<&str>) -> Result<()> {
    let project_root = resolve_project(project)?;
    let target = resolve_memory_file(&project_root, file)?;
    let content = read_memory_content(&target, &[project_root])
        .with_context(|| format!("read memory file {}", target.display()))?;
    if ctx.json {
        let body = json!({
            "path": target,
            "content": content,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    print!("{}", content);
    if !content.ends_with('\n') {
        println!();
    }
    Ok(())
}
