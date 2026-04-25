//! Trash management: `trash list`, `trash restore`, `trash empty`.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

pub fn trash_list_cmd(ctx: &AppContext, older_than: Option<&str>) -> Result<()> {
    use claudepot_core::trash::{self, TrashFilter};
    let filter = TrashFilter {
        older_than: older_than.map(parse_duration).transpose()?,
        kind: None,
    };
    let data_dir = paths::claudepot_data_dir();
    let listing = trash::list(&data_dir, filter).context("list trash")?;
    if ctx.json {
        print_json(&listing);
        return Ok(());
    }
    if listing.entries.is_empty() {
        println!("Trash is empty.");
        return Ok(());
    }
    for e in &listing.entries {
        println!(
            "{}  {:?}  {}  {}",
            e.id,
            e.kind,
            format_size(e.size),
            e.orig_path.display()
        );
    }
    println!(
        "Total: {} entry(ies), {}",
        listing.entries.len(),
        format_size(listing.total_bytes)
    );
    Ok(())
}

pub fn trash_restore_cmd(ctx: &AppContext, id: &str, to: Option<&str>) -> Result<()> {
    use claudepot_core::trash;
    let data_dir = paths::claudepot_data_dir();
    let cwd = to.map(Path::new);
    let restored = trash::restore(&data_dir, id, cwd).context("restore trash")?;
    if ctx.json {
        print_json(&serde_json::json!({ "restored": restored }));
    } else {
        println!("Restored to {}", restored.display());
    }
    Ok(())
}

pub fn trash_empty_cmd(ctx: &AppContext, older_than: Option<&str>) -> Result<()> {
    use claudepot_core::trash::{self, TrashFilter};
    // Refuse on a TTY without --yes.
    if !ctx.yes && atty_like() {
        bail!("`trash empty` requires --yes on a TTY. Pass -y to confirm.");
    }
    let filter = TrashFilter {
        older_than: older_than.map(parse_duration).transpose()?,
        kind: None,
    };
    let data_dir = paths::claudepot_data_dir();
    let freed = trash::empty(&data_dir, filter).context("empty trash")?;
    if ctx.json {
        print_json(&serde_json::json!({ "freed_bytes": freed }));
    } else {
        println!("Emptied. Freed {}.", format_size(freed));
    }
    Ok(())
}

