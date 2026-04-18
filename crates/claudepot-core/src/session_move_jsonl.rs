//! Surgical JSONL rewriters for `session_move`.
//!
//! Two streams, same philosophy:
//!   - Session transcript (`<slug>/<S>.jsonl`) — rewrite every line's
//!     `cwd` field; byte-exact otherwise.
//!   - Top-level `history.jsonl` — rewrite `project` on lines whose
//!     `sessionId` matches; byte-exact otherwise.
//!
//! Both avoid `serde_json::to_string` on the original content — that
//! round-trip reorders keys (the default `serde_json::Map` is backed
//! by `BTreeMap` here). Instead, parse to validate, then do a literal
//! substring splice of the target field's key-value form.

use crate::session_move_types::MoveSessionError;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use uuid::Uuid;

/// Encode a plain `&str` as a JSON string value. Infallible in practice
/// — `serde_json::to_string(&str)` only errors on non-string inputs or
/// transport faults, neither possible here. Wrapped so the rule
/// "never `.unwrap()` in core" (`.claude/rules/rust-conventions.md`) is
/// honored without propagating a panic path to production.
fn encode_json_string(s: &str) -> Result<String, MoveSessionError> {
    serde_json::to_string(s).map_err(|e| std::io::Error::other(e.to_string()).into())
}

/// Stream-copy a JSONL from `src` to `dst`, rewriting the `cwd` field of
/// every object line whose current value equals `from_cwd`. Byte-exact
/// for all other content.
///
/// Returns the number of lines whose `cwd` was rewritten. Lines with a
/// different cwd (mid-session cd, rare but real — CC's own transcript
/// grep found 9/386 sessions in the wild) pass through verbatim.
pub(crate) fn stream_rewrite_jsonl(
    src: &Path,
    dst: &Path,
    from_cwd: &str,
    to_cwd: &str,
) -> Result<usize, MoveSessionError> {
    let parent = dst
        .parent()
        .ok_or_else(|| std::io::Error::other("dst has no parent"))?;
    fs::create_dir_all(parent)?;

    let src_file = fs::File::open(src)?;
    let reader = BufReader::new(src_file);

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    // JSON-encode both values so embedded special chars (quotes,
    // backslashes, control characters) are represented the same way CC's
    // JSON.stringify would write them — that's the form we match against.
    let old_kv = format!(r#""cwd":{}"#, encode_json_string(from_cwd)?);
    let new_kv = format!(r#""cwd":{}"#, encode_json_string(to_cwd)?);

    let mut rewritten = 0usize;
    for line in reader.lines() {
        let line = line?;
        let (out, changed) = rewrite_cwd_in_line(&line, from_cwd, &old_kv, &new_kv)?;
        if changed {
            rewritten += 1;
        }
        writeln!(tmp, "{out}")?;
    }
    tmp.flush()?;
    tmp.persist(dst).map_err(|e| e.error)?;
    Ok(rewritten)
}

/// Rewrite a single JSONL line: if the top-level object has `cwd ==
/// from_cwd`, replace that field's encoded form. Otherwise return the
/// line unchanged. Handles both compact (`"cwd":"…"`) and spaced
/// (`"cwd": "…"`) JSON outputs to accommodate writers that differ from
/// CC's default compact form.
fn rewrite_cwd_in_line(
    line: &str,
    from_cwd: &str,
    old_kv_compact: &str,
    new_kv_compact: &str,
) -> Result<(String, bool), MoveSessionError> {
    // Fast reject: parse first, confirm it's an object with the matching
    // cwd. This guards against a needle appearing inside user-quoted
    // content (message text, tool input, etc.).
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Ok((line.to_string(), false)),
    };
    let Some(obj) = parsed.as_object() else {
        return Ok((line.to_string(), false));
    };
    let Some(cwd_str) = obj.get("cwd").and_then(|v| v.as_str()) else {
        return Ok((line.to_string(), false));
    };
    if cwd_str != from_cwd {
        return Ok((line.to_string(), false));
    }
    // Try compact form first (CC's default).
    if let Some(idx) = line.find(old_kv_compact) {
        let mut out = String::with_capacity(line.len() + new_kv_compact.len());
        out.push_str(&line[..idx]);
        out.push_str(new_kv_compact);
        out.push_str(&line[idx + old_kv_compact.len()..]);
        return Ok((out, true));
    }
    // Spaced form fallback (`"cwd": "…"`).
    let spaced_old = format!(r#""cwd": {}"#, encode_json_string(from_cwd)?);
    // Recover the raw target value from the precomputed compact kv
    // (`"cwd":"<escaped>"`) — parse the value side after the colon.
    let to_value = compact_kv_value(new_kv_compact).unwrap_or_default();
    let spaced_new = format!(r#""cwd": {}"#, encode_json_string(&to_value)?);
    if let Some(idx) = line.find(&spaced_old) {
        let mut out = String::with_capacity(line.len() + spaced_new.len());
        out.push_str(&line[..idx]);
        out.push_str(&spaced_new);
        out.push_str(&line[idx + spaced_old.len()..]);
        return Ok((out, true));
    }
    // Unusual whitespace we don't recognize — parse validated but splice
    // failed. Leave the line alone rather than risk a wrong rewrite.
    Ok((line.to_string(), false))
}

/// Parse a `"key":"<escaped>"` fragment and return the raw string value.
fn compact_kv_value(compact_kv: &str) -> Option<String> {
    let after_colon = compact_kv.splitn(2, ':').nth(1)?;
    serde_json::from_str::<String>(after_colon).ok()
}

/// Rewrite `project` fields in `history.jsonl` for lines whose
/// `sessionId` matches `session_id` AND whose `project` matches
/// `from_cwd`. Returns `(rewritten, unmapped)` where `unmapped` is lines
/// whose `project` matches `from_cwd` but which lack a `sessionId`
/// field — typically pre-sessionId CC writes. Those are left alone
/// (we cannot attribute them to a single session).
///
/// Byte-exact for non-target lines.
pub(crate) fn rewrite_history_jsonl(
    path: &Path,
    session_id: Uuid,
    from_cwd: &str,
    to_cwd: &str,
) -> Result<(usize, usize), MoveSessionError> {
    let contents = fs::read_to_string(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("history.jsonl has no parent"))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;

    let old_kv = format!(r#""project":{}"#, encode_json_string(from_cwd)?);
    let new_kv = format!(r#""project":{}"#, encode_json_string(to_cwd)?);
    let sid_str = session_id.to_string();

    let mut rewritten = 0usize;
    let mut unmapped = 0usize;

    for line in contents.lines() {
        let parsed: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                writeln!(tmp, "{line}")?;
                continue;
            }
        };
        let Some(obj) = parsed.as_object() else {
            writeln!(tmp, "{line}")?;
            continue;
        };
        let project_matches = obj.get("project").and_then(|v| v.as_str()) == Some(from_cwd);
        let sid_matches = obj.get("sessionId").and_then(|v| v.as_str()) == Some(&sid_str);

        if project_matches && sid_matches {
            // Rewrite project field in place.
            if let Some(idx) = line.find(&old_kv) {
                let mut out = String::with_capacity(line.len() + new_kv.len());
                out.push_str(&line[..idx]);
                out.push_str(&new_kv);
                out.push_str(&line[idx + old_kv.len()..]);
                writeln!(tmp, "{out}")?;
                rewritten += 1;
                continue;
            }
            // Fallback: spaced form.
            let old_spaced = format!(r#""project": {}"#, encode_json_string(from_cwd)?);
            let new_spaced = format!(r#""project": {}"#, encode_json_string(to_cwd)?);
            if let Some(idx) = line.find(&old_spaced) {
                let mut out = String::with_capacity(line.len() + new_spaced.len());
                out.push_str(&line[..idx]);
                out.push_str(&new_spaced);
                out.push_str(&line[idx + old_spaced.len()..]);
                writeln!(tmp, "{out}")?;
                rewritten += 1;
                continue;
            }
            // Couldn't splice deterministically; leave as-is.
            writeln!(tmp, "{line}")?;
        } else if project_matches && !obj.contains_key("sessionId") {
            unmapped += 1;
            writeln!(tmp, "{line}")?;
        } else {
            writeln!(tmp, "{line}")?;
        }
    }
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok((rewritten, unmapped))
}
