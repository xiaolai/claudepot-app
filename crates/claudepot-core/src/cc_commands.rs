//! Claude Code's slash commands, as text.
//!
//! Reading `~/.claude/commands`, `<project>/.claude/commands` and the
//! `commands/` directory of every installed plugin, so a surface that
//! can only send **prose** can still offer them.
//!
//! ## Why this exists at all
//!
//! A slash command sent over CC's peer socket does not run. The inbox
//! builds its dispatch as `{…, skipSlashCommands: true, isMeta: true}`
//! and CC's own predicate for "is this a command" is
//! `startsWith("/") && !skipSlashCommands`, so `/audit-fix` arrives as
//! nine characters of prose and Claude answers *about* it. That flag is
//! hardcoded at every injection site; it is not a setting and there is
//! no permission that changes it.
//!
//! **Expanding on our side is what CC does on its own.** CC expands a
//! command at the input layer and then dispatches the *expansion* with
//! `skipSlashCommands: true`, keeping the original `/foo` only in
//! `preExpansionValue` so the transcript can still show what was typed.
//! By the time anything is dispatched it is already plain text. So this
//! is not a way around CC — it is the same step, one layer earlier.
//!
//! ## What an expansion is NOT
//!
//! Running `/foo` through CC does more than paste a body, and the
//! difference is the honest limit of this feature:
//!
//! - **`allowed-tools` does not travel.** 272 of the 732 command files
//!   on the machine this was written for declare one. Invoked properly
//!   the command runs under that restriction; pasted as text it runs
//!   under whatever the *session* allows, which is generally **wider**.
//!   That is the direction that matters, so [`CommandSpec`] carries
//!   `restricts_tools` and every surface must say so.
//! - **`model` does not travel** (40 files). A command that wants a
//!   specific model gets the session's.
//! - Nothing here invokes the Skill tool, sets `argsMayContainSlashCommands`,
//!   or runs `hooks` declared in frontmatter.
//!
//! So: a faithful expansion of the *prompt*, and nothing else. A
//! surface that presents it as "running the command" is lying.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where a command came from. Namespacing follows CC: a plugin command
/// is `<plugin>:<file-stem>`, a user or project one is just the stem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// `<project>/.claude/commands`
    Project,
    /// `~/.claude/commands`
    User,
    /// An installed plugin's `commands/` directory.
    Plugin { plugin: String },
}

/// One command, as the picker needs to describe it.
///
/// The body is **not** here: bodies run to thousands of words apiece
/// and a list of them would be megabytes over a LAN to render a menu.
/// Fetch one with [`expand`] when it is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub source: Source,
    /// Size of the expansion, so a phone can warn before staging 4,000
    /// words into a composer.
    pub body_chars: usize,
    /// Declares `allowed-tools`, which an expansion cannot carry.
    pub restricts_tools: bool,
    /// Declares `model`, ditto.
    pub pins_model: bool,
    /// Not serialized: an absolute path is machine detail the phone has
    /// no use for, and echoing one back as a lookup key would be a
    /// traversal surface. Callers resolve by `name` against a fresh
    /// [`discover`], never by a path a client supplied.
    #[serde(skip)]
    pub path: PathBuf,
}

/// Split YAML front matter from the body.
///
/// Deliberately not a YAML parse: three scalar keys are wanted and
/// pulling in a parser to read them would let a command file's
/// formatting decide whether a menu renders.
fn split_front_matter(text: &str) -> (Vec<(String, String)>, &str) {
    // **CRLF counts.** A command file authored on Windows, or edited by
    // anything that writes CRLF, opens `---\r\n` — so a prefix test for
    // `---\n` alone found no front matter at all. The consequence is not
    // just a missing `description`: the whole YAML block stays in
    // `body`, so it is pasted verbatim into the prompt the panel stages,
    // and `restricts_tools` reads false for a command that does declare
    // `allowed-tools`.
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"));
    let Some(rest) = rest else {
        return (Vec::new(), text);
    };
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), text);
    };
    let (head, tail) = rest.split_at(end);
    let body = tail
        .trim_start_matches("\n---")
        .trim_start_matches('\r')
        .trim_start_matches('\n')
        .trim_start_matches('\r');
    let mut keys = Vec::new();
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.starts_with(char::is_whitespace) || k.is_empty() {
            continue; // a nested value, not a top-level key
        }
        let v = v.trim().trim_matches('"').trim_matches('\'');
        // `str::lines` already drops a trailing `\r`; the trim above
        // covers a value that carried one mid-line.
        keys.push((k.trim().to_string(), v.to_string()));
    }
    (keys, body)
}

/// Put the user's arguments where the command expects them.
///
/// `$ARGUMENTS` is the whole string; `$1`..`$9` are whitespace-split
/// positions. Both are what CC's own command files use — measured 1,068
/// uses of the former and 8 of the latter across this machine.
///
/// An unfilled position becomes empty rather than staying literal: a
/// stray `$2` on screen reads as a bug in the command, and the user
/// cannot tell it came from leaving an argument off.
fn substitute(body: &str, args: &str) -> String {
    let args = args.trim();
    let mut out = body.replace("$ARGUMENTS", args);
    if out.contains('$') {
        let words: Vec<&str> = args.split_whitespace().collect();
        for i in 1..=9usize {
            let needle = format!("${i}");
            if out.contains(&needle) {
                out = out.replace(&needle, words.get(i - 1).copied().unwrap_or(""));
            }
        }
    }
    out
}

fn spec_from(path: &Path, name: String, source: Source) -> Option<CommandSpec> {
    let text = std::fs::read_to_string(path).ok()?;
    let (keys, body) = split_front_matter(&text);
    // A non-empty SCALAR — for the two fields that are rendered.
    let get = |want: &str| {
        keys.iter()
            .find(|(k, _)| k == want)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    // **Presence**, which is a different question. `allowed-tools` is
    // commonly a YAML list:
    //
    //     allowed-tools:
    //       - Bash
    //       - Read
    //
    // …whose first line has an empty scalar. Asking `get` about it
    // therefore answered "absent", so `restricts_tools` was false for a
    // command that plainly does restrict tools — and that flag is the
    // only thing telling the reader the expansion runs under the
    // *session's* permissions rather than the command's own, which are
    // generally narrower. Losing the warning on the commands most
    // likely to need it is the wrong direction to fail in.
    let has = |want: &str| keys.iter().any(|(k, _)| k == want);
    Some(CommandSpec {
        name,
        description: get("description"),
        argument_hint: get("argument-hint"),
        source,
        body_chars: body.chars().count(),
        restricts_tools: has("allowed-tools"),
        pins_model: has("model"),
        path: path.to_path_buf(),
    })
}

fn commands_in(dir: &Path, source: &Source, prefix: Option<&str>, out: &mut Vec<CommandSpec>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    found.sort();
    for path in found {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = match prefix {
            Some(p) => format!("{p}:{stem}"),
            None => stem.to_string(),
        };
        if let Some(spec) = spec_from(&path, name, source.clone()) {
            out.push(spec);
        }
    }
}

/// Plugins whose commands apply to `cwd`, newest install path per
/// plugin name.
///
/// `installed_plugins.json` records one entry per *installation*, so a
/// plugin installed into eleven projects appears eleven times with
/// eleven install paths and possibly eleven versions. Taking them all
/// would offer the same command a dozen times, some from stale
/// versions. A project-scoped entry counts only for its own project.
fn applicable_plugins(cwd: &Path, config_dir: &Path) -> Vec<(String, PathBuf)> {
    let path = config_dir.join("plugins/installed_plugins.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(plugins) = json.get("plugins").and_then(|p| p.as_object()) else {
        return Vec::new();
    };

    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut names: Vec<&String> = plugins.keys().collect();
    names.sort();
    for full in names {
        // `cc-suite@xiaolai` is invoked as `/cc-suite:…`; the
        // marketplace is not part of the command name.
        let short = full.split('@').next().unwrap_or(full).to_string();
        let Some(installs) = plugins.get(full).and_then(|v| v.as_array()) else {
            continue;
        };
        // Project scope first: a project pinning its own version must
        // win over the same plugin installed globally.
        for want_project in [true, false] {
            for inst in installs {
                let scope = inst.get("scope").and_then(|s| s.as_str()).unwrap_or("");
                let is_project = scope == "project";
                if is_project != want_project {
                    continue;
                }
                if is_project && inst.get("projectPath").and_then(|p| p.as_str()) != cwd.to_str() {
                    continue;
                }
                let Some(dir) = inst.get("installPath").and_then(|p| p.as_str()) else {
                    continue;
                };
                if seen.insert(short.clone()) {
                    out.push((short.clone(), PathBuf::from(dir)));
                }
            }
        }
    }
    out
}

/// Every command available to a session working in `cwd`.
///
/// Ordered project, then user, then plugins alphabetically, and
/// deduplicated by name with the first occurrence winning — the same
/// precedence CC applies, so the picker cannot offer a command that
/// resolves to different text than the one the session would run.
pub fn discover(cwd: &Path) -> Vec<CommandSpec> {
    discover_in(cwd, &crate::paths::claude_config_dir())
}

/// The body of [`discover`], with CC's config directory passed in.
///
/// Split out so the tests can build a whole world in a temp directory
/// without setting `CLAUDE_CONFIG_DIR`. A test that mutates a process
/// global is a test that fails when the suite runs in parallel — which
/// these did, and the failure looked like a bug in the lookup rather
/// than in the harness.
pub fn discover_in(cwd: &Path, config_dir: &Path) -> Vec<CommandSpec> {
    let mut out = Vec::new();
    commands_in(
        &cwd.join(".claude/commands"),
        &Source::Project,
        None,
        &mut out,
    );
    commands_in(&config_dir.join("commands"), &Source::User, None, &mut out);
    for (plugin, dir) in applicable_plugins(cwd, config_dir) {
        let source = Source::Plugin {
            plugin: plugin.clone(),
        };
        commands_in(&dir.join("commands"), &source, Some(&plugin), &mut out);
    }

    let mut seen = BTreeSet::new();
    out.retain(|c| seen.insert(c.name.clone()));
    out
}

/// The text a command would have put in the prompt.
///
/// Front matter is stripped — it is instructions for CC, not for
/// Claude, and pasting `allowed-tools:` into a conversation reads as a
/// claim about what is permitted.
pub fn expand(spec: &CommandSpec, args: &str) -> std::io::Result<String> {
    let text = std::fs::read_to_string(&spec.path)?;
    let (_, body) = split_front_matter(&text);
    Ok(substitute(body, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn front_matter_splits_off_and_the_body_starts_clean() {
        let (keys, body) = split_front_matter(
            "---\ndescription: Do the thing\nargument-hint: \"[scope]\"\n---\n\n# Heading\nbody",
        );
        assert_eq!(keys[0], ("description".into(), "Do the thing".into()));
        // Quotes come off, or the hint renders with them on the phone.
        assert_eq!(keys[1], ("argument-hint".into(), "[scope]".into()));
        assert!(
            body.starts_with("\n# Heading") || body.starts_with("# Heading"),
            "{body:?}"
        );
    }

    #[test]
    fn a_file_without_front_matter_is_all_body() {
        // 9 of 732 on the reference machine.
        let (keys, body) = split_front_matter("just instructions\n");
        assert!(keys.is_empty());
        assert_eq!(body, "just instructions\n");
    }

    #[test]
    fn a_horizontal_rule_in_the_body_is_not_front_matter() {
        let text = "Do this.\n\n---\n\nThen that.\n";
        let (keys, body) = split_front_matter(text);
        assert!(keys.is_empty());
        assert_eq!(body, text, "the whole file is the body");
    }

    #[test]
    fn nested_yaml_does_not_become_a_top_level_key() {
        let (keys, _) = split_front_matter("---\nhooks:\n  before: x\ndescription: d\n---\nb");
        assert!(keys.iter().any(|(k, _)| k == "description"));
        assert!(!keys.iter().any(|(k, _)| k == "before"), "{keys:?}");
    }

    #[test]
    fn arguments_land_where_the_command_expects_them() {
        assert_eq!(substitute("run $ARGUMENTS now", " 3 "), "run 3 now");
        assert_eq!(substitute("first=$1 second=$2", "a b"), "first=a second=b");
    }

    #[test]
    fn an_unfilled_position_empties_rather_than_showing_a_dollar_sign() {
        // A literal `$2` on screen reads as a broken command, and the
        // user cannot tell it came from omitting an argument.
        assert_eq!(substitute("x=$1 y=$2", "only"), "x=only y=");
        assert_eq!(substitute("run $ARGUMENTS", ""), "run ");
    }

    #[test]
    fn a_dollar_that_is_not_a_placeholder_survives() {
        // Command bodies quote shell.
        assert_eq!(
            substitute("echo $HOME and $(pwd)", "x"),
            "echo $HOME and $(pwd)"
        );
    }

    #[test]
    fn a_spec_records_what_an_expansion_cannot_carry() {
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "restricted.md",
            "---\ndescription: d\nallowed-tools: Read, Grep\nmodel: opus\n---\nbody here",
        );
        let s = spec_from(&p, "restricted".into(), Source::User).unwrap();
        assert!(s.restricts_tools, "allowed-tools must be flagged");
        assert!(s.pins_model);
        assert_eq!(s.body_chars, "body here".chars().count());
    }

    #[test]
    fn a_plain_command_flags_neither() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "plain.md", "---\ndescription: d\n---\nbody");
        let s = spec_from(&p, "plain".into(), Source::User).unwrap();
        assert!(!s.restricts_tools);
        assert!(!s.pins_model);
    }

    #[test]
    fn the_path_never_crosses_to_a_client() {
        // It is machine detail, and accepting one back as a lookup key
        // would be a traversal surface.
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "x.md", "---\ndescription: d\n---\nbody");
        let s = spec_from(&p, "x".into(), Source::User).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("path"), "{json}");
        assert!(!json.contains(d.path().to_str().unwrap()), "{json}");
    }

    #[test]
    fn expanding_drops_the_front_matter() {
        // `allowed-tools:` pasted into a conversation reads as a claim
        // about what is permitted.
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "c.md",
            "---\ndescription: d\nallowed-tools: Read\n---\nDo $ARGUMENTS please",
        );
        let s = spec_from(&p, "c".into(), Source::User).unwrap();
        let out = expand(&s, "the thing").unwrap();
        assert_eq!(out, "Do the thing please");
        assert!(!out.contains("allowed-tools"));
    }

    #[test]
    fn project_commands_win_over_user_commands_of_the_same_name() {
        let home = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "commands/dup.md",
            "---\ndescription: from user\n---\nU",
        );
        write(
            proj.path(),
            ".claude/commands/dup.md",
            "---\ndescription: from project\n---\nP",
        );

        let found = discover_in(proj.path(), home.path());
        let dup: Vec<_> = found.iter().filter(|c| c.name == "dup").collect();
        assert_eq!(dup.len(), 1, "deduplicated");
        assert_eq!(dup[0].description.as_deref(), Some("from project"));
        assert_eq!(dup[0].source, Source::Project);
    }

    #[test]
    fn a_project_scoped_plugin_does_not_leak_into_another_project() {
        let home = tempfile::tempdir().unwrap();
        let mine = tempfile::tempdir().unwrap();
        let theirs = tempfile::tempdir().unwrap();

        let install = home.path().join("p/only-theirs");
        write(
            &install,
            "commands/secret.md",
            "---\ndescription: d\n---\nbody",
        );
        let manifest = serde_json::json!({
            "version": 2,
            "plugins": {
                "kit@market": [{
                    "scope": "project",
                    "projectPath": theirs.path().to_str().unwrap(),
                    "installPath": install.to_str().unwrap(),
                }]
            }
        });
        write(
            home.path(),
            "plugins/installed_plugins.json",
            &manifest.to_string(),
        );

        assert!(
            discover_in(mine.path(), home.path())
                .iter()
                .all(|c| c.name != "kit:secret"),
            "a plugin pinned to another project must not appear here"
        );
        assert!(
            discover_in(theirs.path(), home.path())
                .iter()
                .any(|c| c.name == "kit:secret"),
            "and must appear in its own"
        );
    }

    #[test]
    fn a_plugin_installed_many_times_is_offered_once() {
        // `installed_plugins.json` records one entry per installation,
        // so a plugin in eleven projects appears eleven times.
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();

        let a = home.path().join("p/v1");
        let b = home.path().join("p/v2");
        write(&a, "commands/go.md", "---\ndescription: v1\n---\nA");
        write(&b, "commands/go.md", "---\ndescription: v2\n---\nB");
        let manifest = serde_json::json!({
            "version": 2,
            "plugins": {
                "kit@market": [
                    {"scope": "user", "installPath": a.to_str().unwrap()},
                    {"scope": "project", "projectPath": cwd.path().to_str().unwrap(),
                     "installPath": b.to_str().unwrap()},
                ]
            }
        });
        write(
            home.path(),
            "plugins/installed_plugins.json",
            &manifest.to_string(),
        );

        let found: Vec<_> = discover_in(cwd.path(), home.path())
            .into_iter()
            .filter(|c| c.name == "kit:go")
            .collect();
        assert_eq!(found.len(), 1, "offered once, not twice");
        // The project's own pin wins, matching what the session runs.
        assert_eq!(found[0].description.as_deref(), Some("v2"));
    }

    #[test]
    fn a_missing_or_corrupt_manifest_is_no_plugins_not_an_error() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        assert!(applicable_plugins(cwd.path(), home.path()).is_empty());

        write(home.path(), "plugins/installed_plugins.json", "{not json");
        assert!(applicable_plugins(cwd.path(), home.path()).is_empty());
    }

    #[test]
    fn a_project_with_nothing_yields_nothing_rather_than_failing() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        assert!(discover_in(cwd.path(), home.path()).is_empty());
    }

    #[test]
    fn crlf_front_matter_is_parsed_like_lf() {
        // A command file authored on Windows opens `---\r\n`. Testing
        // only for `---\n` found no front matter at all, which does not
        // merely lose `description`: the YAML block stays in the body
        // and is pasted verbatim into the prompt the panel stages.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.md");
        std::fs::write(
            &path,
            "---\r\ndescription: Audit the thing\r\nargument-hint: <path>\r\nallowed-tools: Bash, Read\r\nmodel: opus\r\n---\r\nDo the audit on $ARGUMENTS.\r\n",
        )
        .unwrap();
        let spec = spec_from(&path, "audit".into(), Source::Project).expect("spec");
        assert_eq!(spec.description.as_deref(), Some("Audit the thing"));
        assert_eq!(spec.argument_hint.as_deref(), Some("<path>"));
        assert!(spec.restricts_tools);
        assert!(spec.pins_model);

        // And the body no longer carries the front matter.
        let (_, body) = split_front_matter("---\r\ndescription: x\r\n---\r\nBODY HERE\r\n");
        assert!(
            !body.contains("description:"),
            "front matter leaked: {body:?}"
        );
        assert!(body.starts_with("BODY HERE"), "{body:?}");
    }

    #[test]
    fn a_list_valued_allowed_tools_still_counts_as_a_restriction() {
        // The common YAML shape. Its first line has an empty scalar, so
        // a non-empty-value test answered "absent" and the panel
        // dropped the warning that the expansion runs under the
        // session's permissions rather than the command's.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scoped.md");
        std::fs::write(
            &path,
            "---\ndescription: Scoped\nallowed-tools:\n  - Bash\n  - Read\nmodel:\n  - opus\n---\nBody.\n",
        )
        .unwrap();
        let spec = spec_from(&path, "scoped".into(), Source::Project).expect("spec");
        assert!(
            spec.restricts_tools,
            "a list-valued allowed-tools is still a restriction"
        );
        assert!(spec.pins_model);
        // A scalar field with no value stays absent — presence and
        // "has a renderable value" are different questions.
        assert_eq!(spec.argument_hint, None);
    }

    #[test]
    fn a_command_with_no_front_matter_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.md");
        std::fs::write(&path, "Just a body, no front matter.\n").unwrap();
        let spec = spec_from(&path, "plain".into(), Source::Project).expect("spec");
        assert_eq!(spec.description, None);
        assert!(!spec.restricts_tools);
        assert!(!spec.pins_model);
    }
}
