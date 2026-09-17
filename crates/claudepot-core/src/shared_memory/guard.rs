//! Compile an accepted lesson into a **guard** — a grep tripwire in
//! `scripts/repo-invariants.sh`.
//!
//! # Why guards, and why they ship first
//!
//! A directive (a line in AGENTS.md) is advisory, costs tokens on every
//! turn, and the published evidence says auto-generated ones make agents
//! *worse*. A guard is binding (it fails the build), costs zero tokens,
//! and cannot be hurt by context rot. So guards are the unconditional
//! win and directives have to earn their place with an eval.
//!
//! # The one thing that makes generating CI shell safe
//!
//! Auto-generating a grep that runs in CI is dangerous: a bad pattern
//! either blocks every push (false positive) or gives false confidence
//! (false negative). The safety property that tames it:
//!
//! > **A guard that fires on the current, clean tree is wrong by
//! > construction.** The lesson was already fixed — that is why it is an
//! > accepted lesson — so the anti-pattern should NOT be present now.
//!
//! [`GuardSpec::render`] produces the block; [`super::compile`] writes
//! it, runs the script, and *keeps it only if the tree is still clean*.
//! A generated guard that trips immediately is discarded, not committed.
//!
//! That check alone cannot tell a correct guard from one that **can never
//! fire**, or from one that fires on the wrong files — both leave a clean
//! tree clean. Three shipped that way: one whose `--include` abutted the
//! redirect; `no-mktemp-d-windows-ci`, whose
//! `--include='.github/workflows/*.yml'` could match nothing because grep
//! matches `--include` against a file's *name*; and every generated guard,
//! whose `--include` sat after `--`, where it is an operand, so no glob
//! ever filtered anything. So a spec also carries a [`Witness`] — one line
//! of the anti-pattern at a path the guard claims to cover — and the
//! compiler refuses a guard that does not catch it. A path-shaped glob is
//! split into a search root plus a name glob, the only form grep honours,
//! and includes are rendered before `--`. The third defect was found by
//! the witness check on its first run.
//! This module owns the rendering and the marker bookkeeping; the
//! run-and-check is [`super::compile`]'s, because running a subprocess
//! is I/O.

use serde::{Deserialize, Serialize};

/// The delimiters around the generated region of `repo-invariants.sh`.
/// Everything between them is owned by the compiler and regenerated;
/// everything outside is hand-authored and never touched.
pub const GUARD_BEGIN: &str = "# ─── BEGIN claudepot-generated guards ───";
pub const GUARD_END: &str = "# ─── END claudepot-generated guards ───";

/// A guard the model proposed from an accepted lesson.
///
/// Deliberately small and declarative. The model does not write shell —
/// it fills in these fields, and [`Self::render`] turns them into a
/// tripwire whose shape matches the hand-authored guards already in
/// `repo-invariants.sh`. A model that can emit arbitrary shell is a
/// model that can emit `rm -rf`; a model that fills a struct cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardSpec {
    /// Stable slug, e.g. `no-canonicalize-without-simplify`. Used as the
    /// guard's comment header and its dedup key.
    pub slug: String,
    /// One line: what this guard enforces and why.
    pub rationale: String,
    /// The anti-pattern, as an extended-regex (`grep -rnE`). Its PRESENCE
    /// is the violation. Must not match the current clean tree.
    pub detect_regex: String,
    /// Globs to search (`--include`). Empty = all files.
    pub include_globs: Vec<String>,
    /// Substrings; a match on any is exempt (the legitimate call sites).
    pub allow_substrings: Vec<String>,
    /// Printed when the guard fires, telling the reader what to do.
    pub message: String,
    /// The lesson id this was compiled from. Traces the guard back to its
    /// origin and its provenance.
    pub source_lesson_id: String,
    /// One concrete instance of the anti-pattern. The compiler plants it in
    /// a scratch tree and requires the rendered guard to fire on it — the
    /// only evidence that the guard can fire at all. `None` only for specs
    /// deserialized from before the field existed; those cannot be
    /// installed.
    #[serde(default)]
    pub witness: Option<Witness>,
}

/// A line the guard must catch, and where it would live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// Repo-relative path, matched by `include_globs`.
    pub path: String,
    /// One line of code exhibiting the anti-pattern.
    pub line: String,
}

impl GuardSpec {
    /// Render one `repo-invariants.sh`-style block.
    ///
    /// The shape mirrors the hand-written guards: `grep -rnE` for the
    /// anti-pattern, `grep -v` lines to subtract the allowlist, and an
    /// `::error::` message with `fail=1` on a hit. Single-quoted regex
    /// and globs so the shell doesn't re-interpret them.
    pub fn render(&self) -> Result<String, GuardError> {
        let mut s = String::new();
        // Comment fields are model-controlled. A newline in any of them
        // would END the `#` comment and turn the rest of the line into
        // executable shell that runs in CI — so flatten every comment
        // field to a single line first. `comment_safe` strips CR/LF and
        // other control characters.
        s.push_str(&format!("# guard: {}\n", comment_safe(&self.slug)));
        s.push_str(&format!("# {}\n", comment_safe(&self.rationale)));
        s.push_str(&format!(
            "# compiled from lesson {}\n",
            comment_safe(&self.source_lesson_id)
        ));

        // `--` terminates option parsing so a detect_regex beginning with
        // `-` is treated as a pattern, not a grep flag (which `|| true`
        // would swallow into a silent no-fire).
        let regex = shell_single_quote_inner(&self.detect_regex);
        let greps = search_plan(&self.include_globs)?
            .into_iter()
            .map(|(root, globs)| {
                let root = if root == "." {
                    ".".to_string()
                } else {
                    format!("'{}'", shell_single_quote_inner(&root))
                };
                let includes = globs
                    .iter()
                    .map(|g| format!(" --include='{}'", shell_single_quote_inner(g)))
                    .collect::<String>();
                // Options BEFORE `--`: everything after it is an operand, so
                // an `--include` there is read as a file name that does not
                // exist — its error vanishes into `2>/dev/null` and the filter
                // silently stops filtering. Every generated guard rendered it
                // that way until the witness probe caught one firing on a file
                // its glob excluded.
                format!("grep -rnE{includes} -- '{regex}' {root} 2>/dev/null")
            })
            .collect::<Vec<_>>();
        if greps.len() == 1 {
            s.push_str(&format!("violators=$({} || true)\n", greps[0]));
        } else {
            s.push_str(&format!(
                "violators=$({{ {}; }} || true)\n",
                greps.join("; ")
            ));
        }
        for allow in &self.allow_substrings {
            // `-F` = fixed string (the field is documented as a substring,
            // and an unescaped `.` as a regex would filter everything);
            // `--` guards a leading `-`.
            s.push_str(&format!(
                "violators=$(echo \"$violators\" | grep -Fv -- '{}' || true)\n",
                shell_single_quote_inner(allow),
            ));
        }
        // `grep -v` on an empty string emits one empty line; treat a
        // whitespace-only result as no violation.
        s.push_str("if [ -n \"$(echo \"$violators\" | tr -d '[:space:]')\" ]; then\n");
        s.push_str(&format!(
            "  echo \"::error::{}\"\n",
            shell_double_quote_inner(&self.message),
        ));
        s.push_str("  echo \"$violators\"\n");
        s.push_str("  fail=1\n");
        s.push_str("fi\n");
        Ok(s)
    }

    /// A standalone script that runs only this guard and exits non-zero
    /// when it fires. [`super::compile`] runs it inside a scratch tree
    /// holding the [`Witness`].
    pub fn witness_probe(&self) -> Result<String, GuardError> {
        Ok(format!("fail=0\n{}exit \"$fail\"\n", self.render()?))
    }
}

/// Group `include_globs` by the directory grep has to be started from.
///
/// grep's `--include` is matched against a file's *name*, so
/// `.github/workflows/*.yml` matches nothing — the directory has to become
/// the search root and only `*.yml` stays a glob. `**/` segments are
/// dropped because `-r` already recurses. A wildcard left in the directory
/// part cannot be expressed as a root at all, and is refused rather than
/// rendered into a guard that never fires.
fn search_plan(globs: &[String]) -> Result<Vec<(String, Vec<String>)>, GuardError> {
    let mut plan: Vec<(String, Vec<String>)> = Vec::new();
    if globs.is_empty() {
        plan.push((".".to_string(), Vec::new()));
        return Ok(plan);
    }
    for glob in globs {
        let g = glob.trim().trim_start_matches("./");
        let (dir, name) = match g.rfind('/') {
            Some(i) => (&g[..i], &g[i + 1..]),
            None => ("", g),
        };
        if name.is_empty() {
            return Err(GuardError::UnsupportedGlob(glob.clone()));
        }
        let root: Vec<&str> = dir
            .split('/')
            .filter(|seg| !seg.is_empty() && *seg != "**" && *seg != ".")
            .collect();
        if root
            .iter()
            .any(|seg| seg.contains(['*', '?', '[', '{']) || *seg == "..")
        {
            return Err(GuardError::UnsupportedGlob(glob.clone()));
        }
        let root = if root.is_empty() {
            ".".to_string()
        } else {
            root.join("/")
        };
        match plan.iter_mut().find(|(r, _)| *r == root) {
            Some((_, names)) => names.push(name.to_string()),
            None => plan.push((root, vec![name.to_string()])),
        }
    }
    Ok(plan)
}

/// Splice a rendered guard into the script body between the markers,
/// replacing any existing block with the same slug.
///
/// Returns the new script text. If the markers are absent, they are
/// inserted just before the final `if [ "$fail" -ne 0 ]` epilogue so the
/// generated guards run with the hand-written ones. Idempotent per slug:
/// recompiling a lesson replaces its block rather than appending a copy.
pub fn splice_into_script(script: &str, spec: &GuardSpec) -> Result<String, GuardError> {
    let block = spec.render()?;

    // Extract the current generated region (or an empty one).
    let (before, region, after) = match (script.find(GUARD_BEGIN), script.find(GUARD_END)) {
        (Some(b), Some(e)) if e > b => {
            let region_start = b + GUARD_BEGIN.len();
            (
                script[..b].to_string(),
                script[region_start..e].to_string(),
                script[e + GUARD_END.len()..].to_string(),
            )
        }
        (None, None) => {
            // No region yet. Insert one before the fail epilogue.
            let anchor = script
                .find("if [ \"$fail\" -ne 0 ]")
                .ok_or(GuardError::NoEpilogue)?;
            (
                script[..anchor].to_string(),
                String::new(),
                format!("\n{}", &script[anchor..]),
            )
        }
        _ => return Err(GuardError::UnbalancedMarkers),
    };

    // Rebuild the region: keep every block except one with this slug,
    // then append the new one.
    let mut blocks = parse_blocks(&region);
    blocks.retain(|b| b.slug != spec.slug);
    blocks.push(RenderedBlock {
        slug: spec.slug.clone(),
        body: block,
    });

    let mut region_out = String::from("\n");
    for b in &blocks {
        region_out.push_str(&b.body);
        if !b.body.ends_with('\n') {
            region_out.push('\n');
        }
        region_out.push('\n');
    }

    Ok(format!(
        "{before}{GUARD_BEGIN}{region_out}{GUARD_END}{after}"
    ))
}

/// How many generated guards the script currently carries.
pub fn count_generated(script: &str) -> usize {
    match (script.find(GUARD_BEGIN), script.find(GUARD_END)) {
        (Some(b), Some(e)) if e > b => parse_blocks(&script[b + GUARD_BEGIN.len()..e]).len(),
        _ => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    #[error("repo-invariants.sh has no `if [ \"$fail\" -ne 0 ]` epilogue to anchor generated guards before")]
    NoEpilogue,
    #[error(
        "repo-invariants.sh has an unbalanced generated-guard marker (one of BEGIN/END is missing)"
    )]
    UnbalancedMarkers,
    #[error(
        "include glob `{0}` has a wildcard in its directory part; grep can only search from a \
         fixed directory, so this guard could never fire"
    )]
    UnsupportedGlob(String),
}

struct RenderedBlock {
    slug: String,
    body: String,
}

/// Split a generated region into blocks by their `# guard: <slug>`
/// header. Whitespace between blocks is discarded and re-added on render.
fn parse_blocks(region: &str) -> Vec<RenderedBlock> {
    let mut blocks = Vec::new();
    let mut cur: Option<RenderedBlock> = None;
    for line in region.lines() {
        if let Some(slug) = line.strip_prefix("# guard: ") {
            if let Some(b) = cur.take() {
                blocks.push(b);
            }
            cur = Some(RenderedBlock {
                slug: slug.trim().to_string(),
                body: format!("{line}\n"),
            });
        } else if let Some(b) = cur.as_mut() {
            b.body.push_str(line);
            b.body.push('\n');
        }
        // Lines before the first header (blank separators) are dropped.
    }
    if let Some(b) = cur.take() {
        blocks.push(b);
    }
    blocks
}

/// Escape a `'` for inclusion inside a single-quoted shell string.
fn shell_single_quote_inner(s: &str) -> String {
    s.replace('\'', r"'\''")
}

/// Flatten a model-controlled string to a single comment-safe line:
/// strip every control character (including CR/LF) so it cannot end the
/// `#` comment and inject shell into a CI script. Collapses runs of
/// stripped whitespace to single spaces.
fn comment_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// A double-quoted echo argument: neutralize `"`, `` ` ``, `$`, `\`.
fn shell_double_quote_inner(s: &str) -> String {
    s.replace('\\', r"\\")
        .replace('"', r#"\""#)
        .replace('`', r"\`")
        .replace('$', r"\$")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GuardSpec {
        GuardSpec {
            slug: "no-bare-canonicalize".into(),
            rationale: "canonicalize() must be paired with simplify_windows_path".into(),
            detect_regex: r"\.canonicalize\(\)".into(),
            include_globs: vec!["*.rs".into()],
            allow_substrings: vec!["path_utils.rs".into(), "resolve_path".into()],
            message: "bare canonicalize() — pair it with simplify_windows_path".into(),
            source_lesson_id: "abc-123".into(),
            witness: Some(Witness {
                path: "src/lib.rs".into(),
                line: "let p = dir.canonicalize()?;".into(),
            }),
        }
    }

    /// The generated guard that shipped unable to fire, with the two
    /// defects fixed: a path-shaped glob, and a regex that skips the YAML
    /// comment in ci.yml warning against the very pattern it hunts.
    fn mktemp_guard() -> GuardSpec {
        GuardSpec {
            slug: "no-mktemp-d-windows-ci".into(),
            rationale: "mktemp -d under Git Bash on windows-latest returns POSIX /tmp paths; Rust \
                        resolves against current drive, creating foreign-path bugs. Use \
                        ${{ runner.temp }}, which GitHub Actions expands to native OS temp path."
                .into(),
            detect_regex: r"^[^#]*mktemp[[:space:]]+-d".into(),
            include_globs: vec![
                ".github/workflows/*.yml".into(),
                ".github/workflows/*.yaml".into(),
            ],
            allow_substrings: vec![],
            message: "Workflow uses mktemp -d, which breaks on Windows (Git Bash returns POSIX \
                      /tmp). Replace with ${{ runner.temp }} in step env, then reference via \
                      $TEMP_DIR or similar. See rules/paths.md for multi-OS path handling."
                .into(),
            source_lesson_id: "8cd1c1e6-944e-4488-973f-e0bac7bd51a5".into(),
            witness: Some(Witness {
                path: ".github/workflows/ci.yml".into(),
                line: "        run: TMP=$(mktemp -d)".into(),
            }),
        }
    }

    #[test]
    fn a_rendered_guard_is_valid_shell_that_traces_to_its_lesson() {
        let out = spec().render().unwrap();
        assert!(out.contains("grep -rnE --include='*.rs' -- '\\.canonicalize\\(\\)' ."));
        assert!(out.contains("grep -Fv -- 'path_utils.rs'"));
        assert!(out.contains("fail=1"));
        assert!(out.contains("compiled from lesson abc-123"));
    }

    #[test]
    fn a_newline_in_a_comment_field_cannot_inject_shell() {
        // The renderer writes rationale/slug/lesson-id as `#` comments.
        // A model (or a prompt-injected one) that puts a newline in any
        // of them would escape the comment into executable CI shell.
        let mut s = spec();
        s.rationale = "harmless\nrm -rf ~ # oops".into();
        s.slug = "evil\nfail=1".into();
        s.source_lesson_id = "id\necho pwned".into();
        let out = s.render().unwrap();
        // Every comment stays a single `#` line — no bare `rm -rf`,
        // `echo pwned`, or injected `fail=1` on its own line.
        for line in out.lines() {
            if line.contains("rm -rf") || line.contains("echo pwned") {
                assert!(
                    line.trim_start().starts_with('#'),
                    "injected content escaped the comment: {line:?}"
                );
            }
        }
        assert!(!out.contains("\n rm -rf"));
        assert!(!out.contains("\necho pwned"));
    }

    #[test]
    fn a_detect_regex_starting_with_dash_is_not_read_as_a_grep_flag() {
        let mut s = spec();
        s.detect_regex = "-x-marks-it".into();
        let out = s.render().unwrap();
        assert!(out.contains("-- '-x-marks-it'"));
    }

    #[test]
    fn includes_precede_the_option_terminator_and_nothing_abuts_the_redirect() {
        // Two ways this line has silently stopped working. A proposal once
        // rendered `--include='*.yaml'2>/dev/null` — a malformed glob plus
        // a redirect. And every include used to sit AFTER `--`, where it is
        // an operand: a missing file whose error the redirect swallowed.
        let out = spec().render().unwrap();
        let dashes = out.find(" -- ").expect("an option terminator");
        let include = out.find("--include=").expect("an include");
        assert!(include < dashes, "includes must come before `--`:\n{out}");
        assert!(out.contains(" . 2>/dev/null"), "{out}");
        assert!(!out.contains("'2>"), "nothing may abut the redirect");
    }

    #[test]
    fn a_guard_with_no_includes_still_renders_valid_grep() {
        let mut s = spec();
        s.include_globs = vec![];
        let out = s.render().unwrap();
        assert!(out.contains("grep -rnE -- '\\.canonicalize\\(\\)' . 2>/dev/null"));
    }

    #[test]
    fn a_regex_with_a_single_quote_is_escaped_not_broken() {
        let mut s = spec();
        s.detect_regex = r"it's".into();
        let out = s.render().unwrap();
        // The `'\''` dance keeps the shell string intact.
        assert!(out.contains(r"it'\''s"));
    }

    #[test]
    fn a_message_cannot_break_out_of_its_echo() {
        let mut s = spec();
        s.message = r#"bad "$(rm -rf /)" hah"#.into();
        let out = s.render().unwrap();
        // Every `$` is escaped to `\$`, so the shell sees a literal
        // dollar and never performs the command substitution. Assert on
        // the escape, not on substring-absence (the chars are still
        // present — that's the point; they're just inert).
        assert!(
            out.contains(r"\$(rm -rf /)"),
            "the $ must be backslash-escaped"
        );
        assert!(
            !out.contains(r#""$(rm"#),
            "an unescaped \"$( would be executable"
        );
        assert!(
            out.contains(r#"\"\$(rm"#),
            "both the quote and the dollar are neutralized"
        );
    }

    #[test]
    fn splicing_creates_the_region_before_the_epilogue() {
        let script =
            "#!/usr/bin/env bash\nset -e\nfail=0\n\nif [ \"$fail\" -ne 0 ]; then\n  exit 1\nfi\n";
        let out = splice_into_script(script, &spec()).unwrap();
        assert!(out.contains(GUARD_BEGIN));
        assert!(out.contains(GUARD_END));
        // The generated region sits BEFORE the epilogue so it runs.
        assert!(out.find(GUARD_BEGIN).unwrap() < out.find("if [ \"$fail\"").unwrap());
        assert_eq!(count_generated(&out), 1);
    }

    #[test]
    fn recompiling_the_same_slug_replaces_rather_than_duplicates() {
        let script = "fail=0\nif [ \"$fail\" -ne 0 ]; then\n  exit 1\nfi\n";
        let out1 = splice_into_script(script, &spec()).unwrap();

        let mut updated = spec();
        updated.message = "a better message".into();
        let out2 = splice_into_script(&out1, &updated).unwrap();

        assert_eq!(count_generated(&out2), 1, "same slug must not duplicate");
        assert!(out2.contains("a better message"));
        assert!(!out2.contains("bare canonicalize()"));
    }

    #[test]
    fn two_different_slugs_coexist() {
        let script = "fail=0\nif [ \"$fail\" -ne 0 ]; then\n  exit 1\nfi\n";
        let out1 = splice_into_script(script, &spec()).unwrap();
        let mut other = spec();
        other.slug = "another-guard".into();
        let out2 = splice_into_script(&out1, &other).unwrap();
        assert_eq!(count_generated(&out2), 2);
    }

    #[test]
    fn hand_written_content_outside_the_markers_is_untouched() {
        let script = "fail=0\n# hand-written guard 1\nviolators=$(grep foo)\n\nif [ \"$fail\" -ne 0 ]; then\n  exit 1\nfi\n";
        let out = splice_into_script(script, &spec()).unwrap();
        assert!(out.contains("# hand-written guard 1"));
        assert!(out.contains("grep foo"));
    }

    #[test]
    fn an_unbalanced_marker_is_an_error_not_a_silent_overwrite() {
        let script = format!(
            "fail=0\n{GUARD_BEGIN}\nhalf a region\nif [ \"$fail\" -ne 0 ]; then\n  exit 1\nfi\n"
        );
        assert_eq!(
            splice_into_script(&script, &spec()),
            Err(GuardError::UnbalancedMarkers)
        );
    }

    #[test]
    fn a_path_shaped_glob_becomes_a_search_root() {
        // grep matches --include against a file NAME. Rendered verbatim,
        // `.github/workflows/*.yml` matched nothing and the guard could
        // never fire.
        let out = mktemp_guard().render().unwrap();
        assert!(
            out.contains(
                "grep -rnE --include='*.yml' --include='*.yaml' -- \
                 '^[^#]*mktemp[[:space:]]+-d' '.github/workflows' 2>/dev/null"
            ),
            "{out}"
        );
        assert!(!out.contains("--include='.github"), "{out}");
    }

    #[test]
    fn globstar_segments_are_dropped_because_grep_already_recurses() {
        assert_eq!(
            search_plan(&["src/**/*.ts".into(), "**/*.rs".into()]).unwrap(),
            vec![
                ("src".to_string(), vec!["*.ts".to_string()]),
                (".".to_string(), vec!["*.rs".to_string()]),
            ]
        );
    }

    #[test]
    fn a_wildcard_directory_is_refused_rather_than_rendered_inert() {
        for bad in ["crates/*/src/*.rs", "src/", "../outside/*.rs"] {
            assert_eq!(
                search_plan(&[bad.into()]),
                Err(GuardError::UnsupportedGlob(bad.into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn globs_under_different_roots_render_one_grep_each() {
        let mut s = spec();
        s.include_globs = vec!["web/*.ts".into(), "*.rs".into()];
        let out = s.render().unwrap();
        assert!(
            out.contains("violators=$({ grep -rnE --include='*.ts' -- "),
            "{out}"
        );
        assert!(
            out.contains(" 'web' 2>/dev/null; grep -rnE --include='*.rs' -- "),
            "{out}"
        );
        assert!(out.contains(" . 2>/dev/null; } || true)"), "{out}");
    }

    /// Behavioural, not textual: plant files and run the rendered probe.
    #[cfg(unix)]
    fn probe_fires(spec: &GuardSpec, files: &[(&str, &str)]) -> bool {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        let status = std::process::Command::new("bash")
            .arg("-c")
            .arg(spec.witness_probe().unwrap())
            .current_dir(dir.path())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        !status.success()
    }

    #[cfg(unix)]
    #[test]
    fn the_mktemp_guard_fires_on_a_workflow_and_nowhere_else() {
        let g = mktemp_guard();
        let w = g.witness.clone().unwrap();
        assert!(probe_fires(&g, &[(w.path.as_str(), w.line.as_str())]));
        assert!(probe_fires(
            &g,
            &[(".github/workflows/release.yaml", "  - run: D=$(mktemp  -d)")]
        ));
        // The comment in ci.yml that warns against the pattern.
        assert!(!probe_fires(
            &g,
            &[(
                ".github/workflows/ci.yml",
                "  # Do NOT compute this with `mktemp -d`"
            )]
        ));
        // Outside the workflows directory it is not this guard's business.
        assert!(!probe_fires(
            &g,
            &[("scripts/preflight.sh", "T=$(mktemp -d)")]
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_include_filter_actually_filters() {
        // The spec only covers `*.yml`; a match in a `.rs` file is not this
        // guard's business. With the include after `--` it fired anyway.
        let mut s = spec();
        s.include_globs = vec!["*.yml".into()];
        assert!(!probe_fires(
            &s,
            &[("src/lib.rs", "let p = dir.canonicalize()?;")]
        ));
        assert!(probe_fires(&s, &[("ci.yml", "x: dir.canonicalize()")]));
    }

    #[cfg(unix)]
    #[test]
    fn the_unfixed_mktemp_guard_could_never_fire() {
        // The block as it shipped, kept as the reason for the witness
        // check: same witness, zero hits.
        let shipped = "fail=0\n\
            violators=$(grep -rnE 'mktemp\\s+-d' . --include='.github/workflows/*.yml' \
            --include='.github/workflows/*.yaml' 2>/dev/null || true)\n\
            if [ -n \"$(echo \"$violators\" | tr -d '[:space:]')\" ]; then fail=1; fi\n\
            exit \"$fail\"\n";
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".github/workflows/ci.yml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "        run: TMP=$(mktemp -d)").unwrap();
        let status = std::process::Command::new("bash")
            .arg("-c")
            .arg(shipped)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "the shipped guard unexpectedly fired");
    }

    #[test]
    fn the_committed_mktemp_block_is_what_its_spec_renders() {
        // Keeps the block in scripts/repo-invariants.sh reproducible from
        // the spec above, so a renderer change cannot leave the committed
        // guard behind in its old, inert form.
        let block = mktemp_guard().render().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/repo-invariants.sh");
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(
            script.contains(&block),
            "scripts/repo-invariants.sh no longer carries the block this spec renders:\n{block}"
        );
    }
}
