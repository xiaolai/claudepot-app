//! Read-only `agent list` / `agent show` verbs.
//!
//! Both are pure introspection: they never mutate the store and
//! never write a scheduler artifact. They *do* read the active
//! scheduler's registration list, because a stored
//! `lifecycle = installed` is a claim, not a fact — an agent whose
//! artifact is missing will never fire, and a status surface that
//! prints "installed" for it is lying. `orphan_installed_in`
//! cross-checks the two; agents it flags are marked with `*` in
//! `list` and called out inline in `show`.
//!
//! The check is best-effort and never fails the command: an
//! unsupported host or a scheduler query error yields no findings
//! rather than a false accusation (see `orphan_installed_in_using`).
//! It must be handed the already-loaded agent list — re-opening the
//! store while these verbs hold it deadlocks on the store's
//! exclusive advisory lock.
//!
//! They are grouped in one file because they share the same
//! read-the-store-and-format shape; splitting them would fragment
//! closely-related code (see `rules/commands.md`).

use std::collections::HashSet;

use anyhow::{Context, Result};
use claudepot_core::agent::{orphan_installed_in, AgentStore};
use uuid::Uuid;

use super::{agent_to_json, trigger_summary};
use crate::AppContext;

/// Run `agent list` — print every agent with id, name, lifecycle,
/// and a one-line trigger summary.
pub fn list_cmd(ctx: &AppContext) -> Result<()> {
    let store = AgentStore::open().context("opening the agent store")?;
    let agents = store.list();
    let orphans: HashSet<uuid::Uuid> = orphan_installed_in(agents)
        .into_iter()
        .map(|o| o.agent_id)
        .collect();

    if ctx.json {
        let arr: Vec<serde_json::Value> = agents
            .iter()
            .map(|a| with_artifact_status(agent_to_json(a), orphans.contains(&a.id)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if agents.is_empty() {
        println!("No agents.\n\nAn AI client can propose one with `claudepot agent draft`;\nyou install it from Claudepot > Agents.");
        return Ok(());
    }

    let rows: Vec<ListRow> = agents
        .iter()
        .map(|a| ListRow {
            id: a.id.to_string(),
            name: truncate(&a.name, 24),
            lifecycle: list_lifecycle_cell(a.lifecycle, orphans.contains(&a.id)),
            trigger: trigger_summary(a),
        })
        .collect();
    println!("{}", render_list_rows(&rows, orphans.len()));
    Ok(())
}

/// One rendered `agent list` row. Deliberately holds strings rather
/// than an `&Agent`: it keeps the formatter testable without
/// constructing a 35-field fixture that every new `Agent` field
/// would break.
struct ListRow {
    id: String,
    name: String,
    lifecycle: &'static str,
    trigger: String,
}

/// Render the `agent list` table from pre-built rows. Pure.
fn render_list_rows(rows: &[ListRow], orphan_count: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<38}  {:<24}  {:<10}  {}\n",
        "ID", "NAME", "LIFECYCLE", "TRIGGER"
    ));
    out.push_str(&format!(
        "  {:<38}  {:<24}  {:<10}  {}\n",
        "─".repeat(36),
        "─".repeat(22),
        "─".repeat(9),
        "───────"
    ));
    for r in rows {
        out.push_str(&format!(
            "  {:<38}  {:<24}  {:<10}  {}\n",
            r.id, r.name, r.lifecycle, r.trigger,
        ));
    }
    out.push_str(&format!("\n{} agent(s).", rows.len()));
    if orphan_count > 0 {
        out.push_str(&format!(
            "\n\n  * {orphan_count} agent(s) marked installed have NO scheduler artifact —\n    \
             they will never fire. Re-install from Claudepot > Agents to\n    \
             materialize the artifact, or remove the record."
        ));
    }
    out
}

/// The LIFECYCLE cell for one `agent list` row. The `*` marks an
/// agent whose stored lifecycle claims `installed` but which the
/// scheduler has no artifact for.
fn list_lifecycle_cell(
    lifecycle: claudepot_core::agent::Lifecycle,
    artifact_missing: bool,
) -> &'static str {
    match lifecycle {
        claudepot_core::agent::Lifecycle::Draft => "draft",
        claudepot_core::agent::Lifecycle::Installed if artifact_missing => "installed*",
        claudepot_core::agent::Lifecycle::Installed => "installed",
    }
}

/// The `lifecycle:` line for `agent show`. Spelled out rather than
/// marked, because `show` has room and no legend.
fn show_lifecycle_label(
    lifecycle: claudepot_core::agent::Lifecycle,
    artifact_missing: bool,
) -> &'static str {
    match lifecycle {
        claudepot_core::agent::Lifecycle::Draft => "draft (inert — install in the GUI to arm)",
        claudepot_core::agent::Lifecycle::Installed if artifact_missing => {
            "installed — BUT NO SCHEDULER ARTIFACT EXISTS; it will never fire"
        }
        claudepot_core::agent::Lifecycle::Installed => "installed",
    }
}

/// Overwrite the `scheduler_artifact_missing` default that
/// [`agent_to_json`] emits, with the result of the scheduler
/// cross-check.
///
/// The key is `true` only for a positive finding: the agent claims
/// `installed`, its trigger needs an OS artifact, it is enabled, and
/// the scheduler reports no Claudepot-managed artifact for it. It is
/// `false` for every other case — including "not applicable"
/// (manual/event triggers, disabled agents, drafts) and "could not
/// determine" (unsupported host, scheduler query failed). Treat
/// `true` as actionable and `false` as "nothing to report".
fn with_artifact_status(mut v: serde_json::Value, missing: bool) -> serde_json::Value {
    if let Some(obj) = v.as_object_mut() {
        obj.insert("scheduler_artifact_missing".into(), missing.into());
    }
    v
}

/// Run `agent show <id>` — print one agent's full spec. `id` may be
/// an agent UUID or a name (resolved via the store's name index).
pub fn show_cmd(ctx: &AppContext, id_or_name: &str) -> Result<()> {
    let store = AgentStore::open().context("opening the agent store")?;
    let target = id_or_name.trim();

    // Accept either a UUID or a name — `show` is a human-driven
    // verb and names are easier to type than uuids.
    let agent = match Uuid::parse_str(target) {
        Ok(uuid) => store.get(&uuid),
        Err(_) => store.get_by_name(target),
    }
    .with_context(|| format!("no agent matching '{id_or_name}'"))?;

    // Cross-check just this agent. `orphan_installed_in` takes a
    // slice, so pass a one-element view rather than re-deriving the
    // predicate here.
    let artifact_missing = !orphan_installed_in(std::slice::from_ref(agent)).is_empty();

    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&with_artifact_status(
                agent_to_json(agent),
                artifact_missing
            ))?
        );
        return Ok(());
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Agent: {}\n",
        agent.display_name.as_deref().unwrap_or(&agent.name)
    ));
    out.push_str(&format!("  id:           {}\n", agent.id));
    out.push_str(&format!("  name:         {}\n", agent.name));
    let lifecycle = show_lifecycle_label(agent.lifecycle, artifact_missing);
    out.push_str(&format!("  lifecycle:    {lifecycle}\n"));
    if let Some(by) = &agent.drafted_by {
        out.push_str(&format!("  drafted-by:   {by}\n"));
    }
    out.push_str(&format!("  enabled:      {}\n", agent.enabled));
    if let Some(d) = &agent.description {
        out.push_str(&format!("  description:  {d}\n"));
    }
    out.push_str(&format!(
        "  model:        {}\n",
        agent.model.as_deref().unwrap_or("(CLI default)")
    ));
    out.push_str(&format!("  cwd:          {}\n", agent.cwd));
    out.push_str(&format!("  trigger:      {}\n", trigger_summary(agent)));
    out.push_str(&format!(
        "  permissions:  {}\n",
        agent.permission_mode.as_cli_flag()
    ));
    if !agent.allowed_tools.is_empty() {
        out.push_str(&format!(
            "  allow-tools:  {}\n",
            agent.allowed_tools.join(", ")
        ));
    }
    if !agent.disallowed_tools.is_empty() {
        out.push_str(&format!(
            "  deny-tools:   {}\n",
            agent.disallowed_tools.join(", ")
        ));
    }
    if !agent.mcp_servers.is_empty() {
        let names: Vec<String> = agent
            .mcp_servers
            .iter()
            .map(|m| match m {
                claudepot_core::agent::McpServerRef::ClaudepotMemory => {
                    "claudepot-memory".to_string()
                }
                claudepot_core::agent::McpServerRef::Custom { name, .. } => name.clone(),
            })
            .collect();
        out.push_str(&format!("  mcp-servers:  {}\n", names.join(", ")));
    }
    if let Some(ra) = &agent.run_as {
        out.push_str(&format!("  run-as:       {ra}\n"));
    }
    if let Some(tb) = agent.task_budget {
        out.push_str(&format!("  task-budget:  {tb} tokens/run\n"));
    }
    if let Some(rl) = &agent.rate_limit {
        let mut parts = Vec::new();
        if let Some(i) = rl.min_interval_secs {
            parts.push(format!("min {i}s between runs"));
        }
        if let Some(d) = rl.max_per_day {
            parts.push(format!("max {d}/day"));
        }
        if !parts.is_empty() {
            out.push_str(&format!("  rate-limit:   {}\n", parts.join(", ")));
        }
    }
    out.push_str(&format!(
        "  output:       {}\n",
        agent.output_format.as_cli_flag()
    ));
    out.push_str("  prompt:\n");
    for line in agent.prompt.lines() {
        out.push_str(&format!("    {line}\n"));
    }
    print!("{out}");
    Ok(())
}

/// Truncate a string to `max` chars, appending an ellipsis when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_leaves_short_strings() {
        assert_eq!(truncate("short", 24), "short");
    }

    #[test]
    fn truncate_cuts_long_strings_with_ellipsis() {
        let got = truncate("aaaaaaaaaaaaaaaaaaaaaaaaaaaa", 10);
        assert_eq!(got.chars().count(), 10);
        assert!(got.ends_with('…'));
    }

    // ---- scheduler-artifact status rendering ----
    //
    // This is a status surface: it tells the user whether an agent
    // will actually fire. A wrong cell here is worse than no cell,
    // so the mapping is pinned rather than left to manual checks.

    use claudepot_core::agent::Lifecycle;

    fn row(lifecycle: &'static str) -> ListRow {
        ListRow {
            id: "id".into(),
            name: "name".into(),
            lifecycle,
            trigger: "cron 0 9 * * *".into(),
        }
    }

    #[test]
    fn list_cell_marks_installed_agent_with_no_artifact() {
        assert_eq!(
            list_lifecycle_cell(Lifecycle::Installed, true),
            "installed*"
        );
    }

    #[test]
    fn list_cell_leaves_healthy_installed_agent_unmarked() {
        assert_eq!(
            list_lifecycle_cell(Lifecycle::Installed, false),
            "installed"
        );
    }

    #[test]
    fn list_cell_never_marks_a_draft() {
        // A draft has no artifact by design. Marking it would send
        // the user to "re-install to fix" for a record that is
        // working exactly as intended.
        assert_eq!(list_lifecycle_cell(Lifecycle::Draft, true), "draft");
        assert_eq!(list_lifecycle_cell(Lifecycle::Draft, false), "draft");
    }

    #[test]
    fn list_cell_fits_the_column_width() {
        // The LIFECYCLE column is `{:<10}`; a wider cell would shear
        // the table.
        for cell in [
            list_lifecycle_cell(Lifecycle::Installed, true),
            list_lifecycle_cell(Lifecycle::Installed, false),
            list_lifecycle_cell(Lifecycle::Draft, false),
        ] {
            assert!(cell.chars().count() <= 10, "cell {cell:?} overflows");
        }
    }

    #[test]
    fn list_footer_appears_only_when_something_is_orphaned() {
        // NB: assert on the legend text, not on a bare `*` — the
        // trigger column carries cron expressions full of asterisks.
        let clean = render_list_rows(&[row("installed")], 0);
        assert!(
            !clean.contains("NO scheduler artifact"),
            "no legend without a marked row"
        );
        assert!(!clean.contains("installed*"));
        assert!(clean.contains("1 agent(s)."));

        let flagged = render_list_rows(&[row("installed*")], 1);
        assert!(flagged.contains("NO scheduler artifact"));
        assert!(flagged.contains("* 1 agent(s) marked installed"));
    }

    #[test]
    fn show_label_spells_out_the_missing_artifact() {
        assert_eq!(
            show_lifecycle_label(Lifecycle::Installed, false),
            "installed"
        );

        let broken = show_lifecycle_label(Lifecycle::Installed, true);
        assert!(broken.contains("NO SCHEDULER ARTIFACT"));
        assert!(broken.contains("never fire"));

        // `show` has room for prose, so unlike `list` it must not
        // rely on a bare `*` the user has no legend for.
        assert!(!broken.contains('*'));
        assert!(show_lifecycle_label(Lifecycle::Draft, true).starts_with("draft"));
    }

    #[test]
    fn json_status_key_is_present_both_ways() {
        let base = serde_json::json!({"name": "a"});
        let missing = with_artifact_status(base.clone(), true);
        assert_eq!(
            missing["scheduler_artifact_missing"],
            serde_json::json!(true)
        );
        let fine = with_artifact_status(base, false);
        assert_eq!(fine["scheduler_artifact_missing"], serde_json::json!(false));
    }
}
