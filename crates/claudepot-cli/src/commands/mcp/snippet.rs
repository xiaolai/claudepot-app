//! `print-snippet` / `install-snippet` verb group — both verbs are
//! about the agent-instruction snippet, so they share one file per
//! the commands.md verb-group guidance.
//!
//! Sub-module of `commands/mcp.rs`; see that file's header for the
//! layout rationale.

use super::*;

use claudepot_core::mcp_snippet;

/// Print the snippet to stdout. For users who want to paste it
/// manually instead of using `install-snippet`.
pub fn print_snippet() -> Result<()> {
    print!("{}", snippet_body());
    Ok(())
}

/// Write the snippet to `<claude_config_dir>/claudepot-mcp-instructions.md`
/// (or the override). Idempotent — re-running overwrites the file
/// with the current canonical content. After writing, optionally
/// print the recommended `@`-import line. Path policy, validation,
/// and the import-line format live in `claudepot_core::mcp_snippet::install`
/// — shared with the GUI's Settings → MCP installer so the two
/// can't drift again.
pub fn install_snippet(out: Option<PathBuf>, print_include: bool) -> Result<()> {
    let report = mcp_snippet::install(mcp_snippet::InstallScope::User, None, out.as_deref())
        .context("install snippet")?;
    // Results go to stdout (`rules/commands.md`) — `claudepot mcp
    // install-snippet > log` must capture the written path and the
    // @-import line, not an empty stream.
    println!(
        "Wrote {} ({} bytes)",
        report.path.display(),
        report.bytes_written
    );
    if print_include {
        println!();
        println!("Add this single line to your CLAUDE.md and/or AGENTS.md:");
        println!();
        println!("    {}", report.include_line);
        println!();
        println!("Re-run `claudepot mcp install-snippet` to refresh the snippet content.");
        println!("The @-import line never needs to change.");
    }
    Ok(())
}

// ─── snippet tests ────────────────────────────────────────────

#[cfg(test)]
mod snippet_tests {
    use super::*;

    #[test]
    fn snippet_has_version_header() {
        let body = snippet_body();
        assert!(
            body.contains(&format!(
                "claudepot-mcp-instructions v{}",
                claudepot_core::mcp_snippet::SNIPPET_VERSION
            )),
            "snippet should embed its version stamp"
        );
    }

    #[test]
    fn snippet_mentions_every_user_facing_tool() {
        let body = snippet_body();
        for tool in [
            "claudepot_search_memory",
            "claudepot_read_conversation",
            "claudepot_remember",
            "claudepot_log_decision",
            "claudepot_submit_evidence",
            "claudepot_list_memories",
            "claudepot_list_decisions",
            "claudepot_list_sessions",
            "claudepot_list_projects",
        ] {
            assert!(
                body.contains(tool),
                "snippet should mention {tool} so the agent knows it exists"
            );
        }
    }

    #[test]
    fn snippet_teaches_the_agent_draft_verb() {
        // Phase 2: the installed snippet must teach the AI-drafting
        // path — the `agent draft` verb and the human-only install
        // gate. The canonical assertions live in
        // `mcp_snippet::tests`; this re-checks via the CLI's
        // re-exported `snippet_body` so a drift in the install path
        // is caught here too.
        let body = snippet_body();
        assert!(
            body.contains("claudepot agent draft"),
            "installed snippet must name the `agent draft` verb"
        );
        assert!(
            body.contains("Review & install") || body.contains("inert"),
            "installed snippet must teach that drafts are inert until installed"
        );
    }

    #[test]
    fn install_snippet_writes_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("snippet.md");

        install_snippet(Some(path.clone()), false).unwrap();
        let v1 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(v1, snippet_body());

        // Re-run — should overwrite cleanly with identical content.
        install_snippet(Some(path.clone()), false).unwrap();
        let v2 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(v1, v2, "re-install should produce identical content");
    }
}
