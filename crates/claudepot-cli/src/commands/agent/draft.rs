//! The `agent draft` verb — create an inert **draft** agent.
//!
//! `draft` is the AI-authoring half of the Phase-2 loop. An AI
//! client (via Bash) hands Claudepot a spec — by flags, by a
//! `--from-json` file, or on stdin — and Claudepot writes a record
//! with `lifecycle = Draft`.
//!
//! A draft is **inert by construction**: this handler calls
//! `AgentStore::add` + `save` and *nothing else*. It never
//! resolves a binary, never installs a shim, never registers with
//! launchd / Task Scheduler / systemd. Arming a draft (and so
//! materializing a scheduler artifact) is a human-only action in
//! the Claudepot GUI — see the `agent.rs` entry file's gate notes.

use std::io::Read;

use anyhow::{anyhow, Context, Result};
use claudepot_core::agent::draft::{build_draft, CliOverrides, DraftInput};
use claudepot_core::agent::{AgentStore, PermissionMode, Trigger};

use super::{agent_to_json, emit, trigger_summary};

/// Where the JSON spec comes from. `--from-json <file>` reads a
/// file; `--from-json -` reads stdin; omitting the flag means a
/// flags-only spec.
fn read_spec_json(from_json: Option<&str>) -> Result<Option<String>> {
    match from_json {
        None => Ok(None),
        Some("-") => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading agent spec JSON from stdin")?;
            Ok(Some(buf))
        }
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading agent spec JSON from {path}"))?;
            Ok(Some(raw))
        }
    }
}

/// Map the `--permission-mode` flag string to a [`PermissionMode`].
fn parse_permission_mode(s: &str) -> Result<PermissionMode> {
    match s {
        "default" => Ok(PermissionMode::Default),
        "acceptEdits" => Ok(PermissionMode::AcceptEdits),
        "bypassPermissions" => Ok(PermissionMode::BypassPermissions),
        "dontAsk" => Ok(PermissionMode::DontAsk),
        "plan" => Ok(PermissionMode::Plan),
        "auto" => Ok(PermissionMode::Auto),
        other => Err(anyhow!(
            "unknown --permission-mode '{other}' (expected one of: default, acceptEdits, bypassPermissions, dontAsk, plan, auto)"
        )),
    }
}

/// Split a comma-/space-separated tool list, keeping parenthesized
/// argument patterns (`Bash(git *)`) intact. Mirrors the GUI
/// form's `parseAllowedTools` so the two authoring surfaces agree.
fn parse_tool_list(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in input.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let t = current.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                current.clear();
            }
            c if c.is_whitespace() && depth == 0 => {
                let t = current.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                current.clear();
            }
            c => current.push(c),
        }
    }
    let t = current.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// Flag bundle for `agent draft`, mirroring the clap args in
/// `main.rs`. Grouped into a struct to dodge the
/// `too_many_arguments` lint.
#[derive(Debug, Default)]
pub struct DraftArgs {
    pub from_json: Option<String>,
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub prompt: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub allowed_tools: Option<String>,
    pub disallowed_tools: Option<String>,
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub run_as: Option<String>,
    pub task_budget: Option<u64>,
    pub attach_memory: bool,
    pub drafted_by: String,
}

/// Run `agent draft`. Reads a spec from `--from-json` and/or flags,
/// normalizes it (Claudepot-native or `AgentDefinition`-shaped JSON
/// both accepted — PRD D2), builds a `lifecycle = Draft` agent, and
/// persists it. Prints the new draft's id.
pub fn draft_cmd(json: bool, args: DraftArgs) -> Result<()> {
    // Resolve flag-derived overrides up front so a bad value fails
    // before we touch the store.
    let permission_mode = match args.permission_mode.as_deref() {
        Some(s) => Some(parse_permission_mode(s)?),
        None => None,
    };
    let trigger = match (args.cron.as_deref(), args.timezone.as_deref()) {
        (Some(cron), tz) => Some(Trigger::Cron {
            cron: cron.to_string(),
            timezone: tz.map(str::to_string),
        }),
        // A `--timezone` with no `--cron` is meaningless; reject it
        // rather than silently dropping the value.
        (None, Some(_)) => {
            return Err(anyhow!("--timezone requires --cron"));
        }
        (None, None) => None,
    };
    let allowed_tools = args.allowed_tools.as_deref().map(parse_tool_list);
    let disallowed_tools = args.disallowed_tools.as_deref().map(parse_tool_list);

    let overrides = CliOverrides {
        name: args.name.clone(),
        cwd: args.cwd.clone(),
        display_name: args.display_name.clone(),
        description: args.description.clone(),
        model: args.model.clone(),
        permission_mode,
        trigger,
        allowed_tools,
        disallowed_tools,
        run_as: args.run_as.clone(),
        task_budget: args.task_budget,
        attach_memory: args.attach_memory,
    };

    // The spec body. `--from-json` provides it; without that flag we
    // synthesize a minimal Claudepot-native spec from `--name`,
    // `--cwd`, and `--prompt` (all three then required).
    let spec =
        match read_spec_json(args.from_json.as_deref())? {
            Some(raw) => DraftInput::from_json(&raw)?.normalize(&overrides)?,
            None => {
                let name = args.name.as_deref().ok_or_else(|| {
                    anyhow!("flags-only draft requires --name (or pass --from-json)")
                })?;
                let cwd = args.cwd.as_deref().ok_or_else(|| {
                    anyhow!("flags-only draft requires --cwd (or pass --from-json)")
                })?;
                let prompt = args.prompt.as_deref().ok_or_else(|| {
                    anyhow!("flags-only draft requires --prompt (or pass --from-json)")
                })?;
                let synthetic = serde_json::json!({
                    "name": name,
                    "cwd": cwd,
                    "prompt": prompt,
                });
                DraftInput::from_json(&synthetic.to_string())?.normalize(&overrides)?
            }
        };

    // Build the inert draft record. `build_draft` validates the
    // name shape, the bypassPermissions invariant, env vars, and
    // any cron expression — a draft is rejected here, never handed
    // to a human in a broken state.
    let agent = build_draft(spec, &args.drafted_by, chrono::Utc::now())?;
    let id = agent.id;

    // Persist. `add` re-validates and enforces name/id uniqueness;
    // `save` is an atomic 0600 write. NOTHING else runs — no shim,
    // no scheduler registration. The draft is inert on disk.
    let mut store = AgentStore::open().context("opening the agent store")?;
    store.add(agent.clone()).context("adding the draft")?;
    store.save().context("saving the agent store")?;

    let human = format!(
        "Drafted agent '{}' ({})\n  id:      {}\n  trigger: {}\n  drafted-by: {}\n\nThis is a DRAFT — it is inert and will not run. Open Claudepot\n> Agents and choose \"Review & install\" to arm it.",
        agent.name,
        agent
            .display_name
            .as_deref()
            .unwrap_or(&agent.name),
        id,
        trigger_summary(&agent),
        args.drafted_by,
    );
    emit(json, agent_to_json(&agent), &human)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_list_keeps_paren_patterns() {
        let got = parse_tool_list("Read, Grep, Bash(git status), Bash(cat *)");
        assert_eq!(got, vec!["Read", "Grep", "Bash(git status)", "Bash(cat *)"]);
    }

    #[test]
    fn parse_tool_list_handles_whitespace_and_empties() {
        assert_eq!(parse_tool_list("  Read   Grep  "), vec!["Read", "Grep"]);
        assert!(parse_tool_list("").is_empty());
        assert!(parse_tool_list("   ").is_empty());
    }

    #[test]
    fn parse_permission_mode_known_and_unknown() {
        assert_eq!(
            parse_permission_mode("bypassPermissions").unwrap(),
            PermissionMode::BypassPermissions
        );
        assert!(parse_permission_mode("nonsense").is_err());
    }
}
