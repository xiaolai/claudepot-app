//! The I/O half of guard compilation: propose a [`GuardSpec`] from an
//! accepted lesson via `claude -p`, then install the rendered block
//! into the lesson's `scripts/repo-invariants.sh` under a
//! write→verify→revert transaction.
//!
//! [`super::guard`] owns the pure rendering and splicing; this module
//! owns the subprocesses (`claude -p` to fill the spec, `bash` to run
//! the script) and the file writes. The safety property is the one
//! [`super::guard`] documents: **a guard that fires on the current,
//! clean tree is wrong by construction** — [`StagedGuard::install`]
//! writes the guard, runs the script, and keeps the guard only if the
//! tree stays clean; otherwise it reverts and refuses.
//!
//! Callers run in blocking contexts (sync CLI handlers), so the
//! subprocesses are synchronous on purpose — same rationale as
//! [`super::git`].

use std::path::{Path, PathBuf};

use crate::shared_memory::git;
use crate::shared_memory::guard::{self, GuardError, GuardSpec};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("spawn `claude -p` to propose a guard")]
    SpawnClaude(#[source] std::io::Error),

    #[error("claude -p failed: {stderr}")]
    ClaudeFailed { stderr: String },

    #[error("model did not return a JSON object")]
    NoJsonObject,

    #[error("parse guard proposal")]
    ParseProposal(#[source] serde_json::Error),

    #[error(
        "the model judged this lesson not compilable to a grep guard \
         (it needs human judgment or isn't a detectable pattern). Keep it as a directive or a note."
    )]
    NotCompilable,

    #[error("model returned compilable=true but no detect_regex")]
    MissingDetectRegex,

    #[error("{project} is not a git repository")]
    NotAGitRepo { project: String },

    #[error("{path} does not exist — guards are a repo-invariants.sh feature")]
    NoInvariantsScript { path: String },

    #[error("read {path}")]
    ReadScript {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("splice guard block")]
    Splice(#[source] GuardError),

    #[error("run repo-invariants.sh")]
    RunInvariants(#[source] std::io::Error),

    #[error(
        "{script} is already failing before this guard is added — fix the existing \
         invariant failure first, then re-run compile."
    )]
    BaselineFailing { script: String },

    #[error("write {path}")]
    WriteScript {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("revert bad guard")]
    RevertBadGuard(#[source] std::io::Error),

    #[error(
        "the generated guard fires on the current clean tree — its detect pattern is wrong \
         (it would block every push). Reverted. Pattern was: {pattern}"
    )]
    GuardFiresOnCleanTree { pattern: String },
}

/// Ask the model to fill in a [`GuardSpec`] from a lesson. Structured
/// output; the model never emits shell.
pub fn propose_guard(
    claude_bin: &str,
    claim: &str,
    directive: &str,
    lesson_id: &str,
) -> Result<GuardSpec, CompileError> {
    let schema = r#"{
      "type":"object",
      "properties":{
        "slug":{"type":"string","description":"kebab-case id, e.g. no-bare-canonicalize"},
        "rationale":{"type":"string","description":"one line: what this enforces and why"},
        "detect_regex":{"type":"string","description":"grep -E regex whose PRESENCE is the violation. Must NOT match already-correct code."},
        "include_globs":{"type":"array","items":{"type":"string"},"description":"file globs, e.g. *.rs"},
        "allow_substrings":{"type":"array","items":{"type":"string"},"description":"path substrings that are legitimate exceptions"},
        "message":{"type":"string","description":"what to print when it fires; tell the reader what to do"},
        "compilable":{"type":"boolean","description":"false if this lesson cannot be a grep tripwire"}
      },
      "required":["slug","rationale","detect_regex","message","compilable"]
    }"#;
    let prompt = format!(
        "You turn an accepted engineering lesson into a grep-based CI tripwire.\n\n\
         LESSON: {claim}\nDIRECTIVE: {directive}\n\n\
         Produce a GuardSpec matching the schema. The detect_regex matches the ANTI-pattern \
         (its presence is the bug). It MUST NOT match already-correct code, because the codebase \
         is currently clean. If the lesson is about human judgment, prose, or anything a grep \
         cannot detect, set compilable=false and leave detect_regex empty. Output ONLY the JSON object.\n\n\
         Schema: {schema}"
    );
    let out = std::process::Command::new(claude_bin)
        .arg("-p")
        .arg(prompt)
        .args(["--model", "claude-haiku-4-5"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(CompileError::SpawnClaude)?;
    if !out.status.success() {
        return Err(CompileError::ClaudeFailed {
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let obj = first_json(&raw).ok_or(CompileError::NoJsonObject)?;
    // Models routinely emit regexes with backslashes that aren't valid
    // JSON escapes (`\.`, `\(`, `\d`), which serde rejects outright. This
    // is the single most common way a guard proposal fails to parse, and
    // it's a packaging problem, not a content one — repair it.
    let repaired = repair_json_escapes(obj);
    let v: serde_json::Value =
        serde_json::from_str(&repaired).map_err(CompileError::ParseProposal)?;

    if !v
        .get("compilable")
        .and_then(|c| c.as_bool())
        .unwrap_or(false)
    {
        return Err(CompileError::NotCompilable);
    }
    Ok(GuardSpec {
        slug: sanitize_slug(
            v.get("slug")
                .and_then(|s| s.as_str())
                .unwrap_or("generated-guard"),
        ),
        rationale: v
            .get("rationale")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        detect_regex: v
            .get("detect_regex")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .ok_or(CompileError::MissingDetectRegex)?
            .to_string(),
        include_globs: str_array(&v, "include_globs"),
        allow_substrings: str_array(&v, "allow_substrings"),
        message: v
            .get("message")
            .and_then(|s| s.as_str())
            .unwrap_or("guard fired")
            .to_string(),
        source_lesson_id: lesson_id.to_string(),
    })
}

/// A guard spliced into its target script **in memory** — nothing on
/// disk has changed yet. [`Self::install`] is the write.
///
/// Staging is separate from installing so a caller can surface the
/// locate/read/splice errors (and show the target path) in a
/// propose-only flow without touching the filesystem.
#[derive(Debug)]
pub struct StagedGuard {
    /// The `scripts/repo-invariants.sh` the guard belongs to.
    pub script_path: PathBuf,
    /// The script as it was read — the revert target.
    original: String,
    /// The script with the guard block spliced in.
    spliced: String,
    /// Kept for the false-positive error message.
    detect_regex: String,
}

/// Locate `project`'s `scripts/repo-invariants.sh`, read it, and splice
/// the guard into it in memory.
pub fn stage_guard(project: &str, spec: &GuardSpec) -> Result<StagedGuard, CompileError> {
    let script_path = repo_invariants_path(project)?;
    let original = std::fs::read_to_string(&script_path).map_err(|e| CompileError::ReadScript {
        path: script_path.display().to_string(),
        source: e,
    })?;
    let spliced = guard::splice_into_script(&original, spec).map_err(CompileError::Splice)?;
    Ok(StagedGuard {
        script_path,
        original,
        spliced,
        detect_regex: spec.detect_regex.clone(),
    })
}

impl StagedGuard {
    /// Write the guard, then prove it doesn't false-positive; revert it
    /// if it does. The full transaction:
    ///
    /// 1. Baseline the unmodified script — refuse if it already fails.
    /// 2. Write the spliced script.
    /// 3. Re-run it; if the tree is no longer clean, the guard is wrong
    ///    by construction — restore the original and refuse to keep it.
    pub fn install(&self) -> Result<(), CompileError> {
        // Baseline the script BEFORE adding the guard. If repo-invariants.sh
        // is already failing for an unrelated reason, we must not write the
        // guard and then blame it for a failure it didn't cause.
        if !run_invariants(&self.script_path)? {
            return Err(CompileError::BaselineFailing {
                script: self.script_path.display().to_string(),
            });
        }

        // Write, then PROVE it doesn't false-positive. A generated guard that
        // trips on the current clean tree is wrong by construction — the
        // lesson was already fixed, so its anti-pattern must be absent now. If
        // it fires, the pattern is bad: revert and refuse to keep it.
        std::fs::write(&self.script_path, &self.spliced).map_err(|e| {
            CompileError::WriteScript {
                path: self.script_path.display().to_string(),
                source: e,
            }
        })?;
        let clean = run_invariants(&self.script_path)?;
        if !clean {
            std::fs::write(&self.script_path, &self.original)
                .map_err(CompileError::RevertBadGuard)?;
            return Err(CompileError::GuardFiresOnCleanTree {
                pattern: self.detect_regex.clone(),
            });
        }
        Ok(())
    }
}

fn repo_invariants_path(project: &str) -> Result<PathBuf, CompileError> {
    let root = git::repo_root(Path::new(project)).ok_or_else(|| CompileError::NotAGitRepo {
        project: project.to_string(),
    })?;
    let p = Path::new(&root).join("scripts/repo-invariants.sh");
    if !p.exists() {
        return Err(CompileError::NoInvariantsScript {
            path: p.display().to_string(),
        });
    }
    Ok(p)
}

fn run_invariants(script: &Path) -> Result<bool, CompileError> {
    // Run from the script's own repo root so its relative `grep`s resolve
    // against that project, not the calling process's cwd.
    let cwd = script
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));
    let out = std::process::Command::new("bash")
        .arg(script)
        .current_dir(cwd)
        .output()
        .map_err(CompileError::RunInvariants)?;
    Ok(out.status.success())
}

fn sanitize_slug(s: &str) -> String {
    let cleaned: String = s
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = cleaned.trim_matches('-').to_string();
    if slug.is_empty() {
        "generated-guard".to_string()
    } else {
        slug
    }
}

fn str_array(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// First balanced JSON object in a string (the model may wrap it in
/// prose). Reuses the same brace-counting discipline as the distiller.
fn first_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return s.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Repair invalid JSON string escapes that models emit — chiefly a lone
/// backslash before a char that JSON doesn't recognize as an escape
/// (`\.`, `\(`, `\d` in a regex). Such a backslash is doubled so it
/// parses to a literal backslash, which is what the model meant.
///
/// Only touches backslashes INSIDE string literals; structural JSON is
/// left alone. A backslash that IS a valid escape (`\"`, `\\`, `\n`, …)
/// passes through untouched, as does a `\uXXXX` sequence.
fn repair_json_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_str = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if !in_str {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
            continue;
        }
        if c == '"' {
            in_str = false;
            out.push(c);
            continue;
        }
        if c == '\\' {
            match chars.peek() {
                // Valid JSON escapes — emit the pair verbatim.
                Some(&n) if matches!(n, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') => {
                    out.push('\\');
                    out.push(n);
                    chars.next();
                }
                // Invalid escape (or trailing backslash): double it.
                _ => out.push_str("\\\\"),
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_a_regex_backslash_that_json_rejects() {
        let broken = r#"{"detect_regex":"\.canonicalize\(\)"}"#;
        // serde rejects the raw form...
        assert!(serde_json::from_str::<serde_json::Value>(broken).is_err());
        // ...and accepts the repaired form, preserving the regex.
        let fixed = repair_json_escapes(broken);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["detect_regex"], r"\.canonicalize\(\)");
    }

    #[test]
    fn leaves_valid_escapes_and_structure_untouched() {
        let ok = r#"{"a":"line\nbreak","b":"quote\"here","c":"tab\tstop"}"#;
        let v: serde_json::Value = serde_json::from_str(&repair_json_escapes(ok)).unwrap();
        assert_eq!(v["a"], "line\nbreak");
        assert_eq!(v["b"], "quote\"here");
        assert_eq!(v["c"], "tab\tstop");
    }

    #[test]
    fn a_unicode_escape_survives() {
        let u = r#"{"x":"é"}"#;
        let v: serde_json::Value = serde_json::from_str(&repair_json_escapes(u)).unwrap();
        assert_eq!(v["x"], "é");
    }
}
