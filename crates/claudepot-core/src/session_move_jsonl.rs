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

/// Test-only thin wrapper over [`stream_rewrite_jsonl_with_progress`]
/// — keeps the existing JSONL-rewriter tests focused on rewrite
/// semantics without threading an unused progress callback.
#[cfg(test)]
pub(crate) fn stream_rewrite_jsonl(
    src: &Path,
    dst: &Path,
    from_cwd: &str,
    to_cwd: &str,
) -> Result<usize, MoveSessionError> {
    stream_rewrite_jsonl_with_progress(src, dst, from_cwd, to_cwd, &mut |_, _| {})
}

/// Stream-copy a JSONL from `src` to `dst`, rewriting the `cwd` field of
/// every object line whose current value equals `from_cwd`. Byte-exact
/// for all other content.
///
/// Returns the number of lines whose `cwd` was rewritten. Lines with a
/// different cwd (mid-session cd, rare but real — CC's own transcript
/// grep found 9/386 sessions in the wild) pass through verbatim.
///
/// Fires `on_progress(done, total)` after each line is written. Pass
/// `&mut |_, _| {}` to suppress. `total` is the precounted line count
/// of the source file (a single fast pass before the rewrite).
pub(crate) fn stream_rewrite_jsonl_with_progress(
    src: &Path,
    dst: &Path,
    from_cwd: &str,
    to_cwd: &str,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<usize, MoveSessionError> {
    let parent = dst
        .parent()
        .ok_or_else(|| std::io::Error::other("dst has no parent"))?;
    fs::create_dir_all(parent)?;

    // Precount lines so the sink can report a real total. A second
    // `BufRead` pass is cheap relative to the JSON parse cost of the
    // rewrite pass — we accept the extra read for a determinate UX.
    let total = count_lines(src)?;

    let src_file = fs::File::open(src)?;
    let reader = BufReader::new(src_file);

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    // JSON-encode both values so embedded special chars (quotes,
    // backslashes, control characters) are represented the same way CC's
    // JSON.stringify would write them — that's the form we match against.
    let old_kv = format!(r#""cwd":{}"#, encode_json_string(from_cwd)?);
    let new_kv = format!(r#""cwd":{}"#, encode_json_string(to_cwd)?);

    let mut rewritten = 0usize;
    let mut done = 0usize;
    for line in reader.lines() {
        let line = line?;
        let (out, changed) = rewrite_cwd_in_line(&line, from_cwd, &old_kv, &new_kv)?;
        if changed {
            rewritten += 1;
        }
        writeln!(tmp, "{out}")?;
        done += 1;
        on_progress(done, total);
    }
    tmp.flush()?;
    tmp.persist(dst).map_err(|e| e.error)?;
    Ok(rewritten)
}

/// Cheap line-count pass for progress reporting. `BufRead::lines` is
/// good enough — the file contents are JSONL so embedded newlines are
/// not a concern.
fn count_lines(path: &Path) -> Result<usize, MoveSessionError> {
    let f = fs::File::open(path)?;
    let r = BufReader::new(f);
    let mut n = 0usize;
    for line in r.lines() {
        let _ = line?;
        n += 1;
    }
    Ok(n)
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
    let after_colon = compact_kv.split_once(':')?.1;
    serde_json::from_str::<String>(after_colon).ok()
}

#[cfg(test)]
mod jsonl_tests {
    //! Behavior contract for `stream_rewrite_jsonl` + `rewrite_history_jsonl`:
    //!   - `cwd` / `project` field values are rewritten in place
    //!   - Non-target fields pass through byte-exact (no key reordering)
    //!   - Malformed lines are preserved unchanged
    //!   - Matching needles inside quoted user content are NOT touched
    //!     (the parse-first guard makes this safe)
    //!   - Spaced JSON (`"cwd": "…"`) is handled as well as compact
    //!   - Idempotent under a second call with the same args (no-op)

    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;

    const FROM_CWD: &str = "/tmp/old";
    const TO_CWD: &str = "/tmp/new";

    fn write(path: &Path, body: &str) {
        let mut f = fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    // --- stream_rewrite_jsonl ---------------------------------------------

    #[test]
    fn stream_rewrite_single_matching_line() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        write(
            &src,
            "{\"type\":\"user\",\"cwd\":\"/tmp/old\",\"message\":\"hi\"}\n",
        );

        let n = stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(n, 1);

        let out = fs::read_to_string(&dst).unwrap();
        assert!(out.contains(r#""cwd":"/tmp/new""#));
        assert!(!out.contains(r#""cwd":"/tmp/old""#));
        // Byte-exact preservation of surrounding fields.
        assert!(out.contains(r#""type":"user""#));
        assert!(out.contains(r#""message":"hi""#));
    }

    #[test]
    fn stream_rewrite_preserves_non_matching_cwd() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        // Mid-session cd — cwd doesn't match FROM_CWD, must pass through.
        write(
            &src,
            "{\"cwd\":\"/other\",\"message\":\"x\"}\n{\"cwd\":\"/tmp/old\",\"m\":\"y\"}\n",
        );

        let n = stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(n, 1);

        let out = fs::read_to_string(&dst).unwrap();
        assert!(out.contains(r#""cwd":"/other""#));
        assert!(out.contains(r#""cwd":"/tmp/new""#));
    }

    #[test]
    fn stream_rewrite_malformed_line_passes_through() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        write(
            &src,
            "not json at all\n{\"cwd\":\"/tmp/old\",\"m\":\"y\"}\n",
        );

        let n = stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(n, 1);

        let out = fs::read_to_string(&dst).unwrap();
        assert!(out.starts_with("not json at all\n"));
        assert!(out.contains(r#""cwd":"/tmp/new""#));
    }

    #[test]
    fn stream_rewrite_does_not_touch_needle_in_message_body() {
        // A session whose content happens to contain the string
        // `"cwd":"/tmp/old"` inside a user message must NOT be rewritten
        // because the parse-first guard finds cwd only in the top-level
        // object. Here the top-level cwd differs.
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        let line = r#"{"cwd":"/elsewhere","message":"pasted: \"cwd\":\"/tmp/old\""}"#;
        write(&src, &format!("{line}\n"));

        let n = stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(n, 0);

        let out = fs::read_to_string(&dst).unwrap();
        assert!(out.contains("/elsewhere"));
        // The literal string inside message body stays.
        assert!(out.contains(r#"\"cwd\":\"/tmp/old\""#));
    }

    #[test]
    fn stream_rewrite_spaced_form_fallback() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        // Simulated writer with pretty spacing (uncommon but valid).
        write(&src, "{\"cwd\": \"/tmp/old\", \"m\": \"y\"}\n");

        let n = stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(n, 1);

        let out = fs::read_to_string(&dst).unwrap();
        assert!(out.contains(r#""cwd": "/tmp/new""#));
    }

    #[test]
    fn stream_rewrite_idempotent_on_second_pass() {
        // Running the same rewrite twice must produce 0 changes the
        // second time — nothing matches FROM_CWD any more.
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        write(&src, "{\"cwd\":\"/tmp/old\",\"m\":\"y\"}\n");

        assert_eq!(stream_rewrite_jsonl(&src, &dst, FROM_CWD, TO_CWD).unwrap(), 1);
        // Second pass reads dst (already rewritten) — must be a no-op.
        let dst2 = dir.path().join("dst2.jsonl");
        assert_eq!(stream_rewrite_jsonl(&dst, &dst2, FROM_CWD, TO_CWD).unwrap(), 0);
    }

    #[test]
    fn stream_rewrite_handles_paths_with_special_chars() {
        // Backslashes and quotes in cwd — JSON escaping must match on both
        // sides of the splice. Without encode_json_string this would miss.
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jsonl");
        let dst = dir.path().join("dst.jsonl");
        let from = r"C:\Users\joker\with quote\" ;
        let to = r"C:\Users\joker\clean";
        // JSON-escape the original for the on-disk fixture.
        let encoded_from = serde_json::to_string(from).unwrap();
        write(&src, &format!("{{\"cwd\":{encoded_from},\"m\":\"x\"}}\n"));

        let n = stream_rewrite_jsonl(&src, &dst, from, to).unwrap();
        assert_eq!(n, 1);
        let out = fs::read_to_string(&dst).unwrap();
        let encoded_to = serde_json::to_string(to).unwrap();
        assert!(out.contains(&format!(r#""cwd":{encoded_to}"#)));
    }

    // --- rewrite_history_jsonl --------------------------------------------

    #[test]
    fn history_rewrites_on_sid_and_project_match() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let sid = Uuid::new_v4();
        write(
            &path,
            &format!(
                "{{\"sessionId\":\"{sid}\",\"project\":\"/tmp/old\",\"m\":\"x\"}}\n\
                 {{\"sessionId\":\"{sid}\",\"project\":\"/other\",\"m\":\"y\"}}\n"
            ),
        );

        let (rewritten, unmapped) =
            rewrite_history_jsonl(&path, sid, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(rewritten, 1);
        assert_eq!(unmapped, 0);

        let out = fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#""project":"/tmp/new""#));
        assert!(out.contains(r#""project":"/other""#));
    }

    #[test]
    fn history_unmapped_for_project_match_without_sessionid() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let sid = Uuid::new_v4();
        write(
            &path,
            "{\"project\":\"/tmp/old\",\"legacy\":true}\n",
        );

        let (rewritten, unmapped) =
            rewrite_history_jsonl(&path, sid, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(rewritten, 0);
        assert_eq!(unmapped, 1);

        // Line is preserved byte-exact when unmappable.
        let out = fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#""project":"/tmp/old""#));
    }

    #[test]
    fn history_leaves_other_sessions_alone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let target = Uuid::new_v4();
        let other = Uuid::new_v4();
        write(
            &path,
            &format!(
                "{{\"sessionId\":\"{other}\",\"project\":\"/tmp/old\",\"m\":\"x\"}}\n\
                 {{\"sessionId\":\"{target}\",\"project\":\"/tmp/old\",\"m\":\"y\"}}\n"
            ),
        );

        let (rewritten, unmapped) =
            rewrite_history_jsonl(&path, target, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(rewritten, 1);
        assert_eq!(unmapped, 0);

        let out = fs::read_to_string(&path).unwrap();
        // The other session's project stays pointing at /tmp/old.
        let other_line = out
            .lines()
            .find(|l| l.contains(&other.to_string()))
            .unwrap();
        assert!(other_line.contains(r#""project":"/tmp/old""#));
        let target_line = out
            .lines()
            .find(|l| l.contains(&target.to_string()))
            .unwrap();
        assert!(target_line.contains(r#""project":"/tmp/new""#));
    }

    #[test]
    fn history_preserves_malformed_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let sid = Uuid::new_v4();
        write(
            &path,
            &format!(
                "{{not valid json\n{{\"sessionId\":\"{sid}\",\"project\":\"/tmp/old\"}}\n"
            ),
        );

        let (rewritten, _unmapped) =
            rewrite_history_jsonl(&path, sid, FROM_CWD, TO_CWD).unwrap();
        assert_eq!(rewritten, 1);

        let out = fs::read_to_string(&path).unwrap();
        assert!(out.starts_with("{not valid json\n"));
    }
}

/// Test-only thin wrapper over [`rewrite_history_jsonl_with_progress`].
#[cfg(test)]
pub(crate) fn rewrite_history_jsonl(
    path: &Path,
    session_id: Uuid,
    from_cwd: &str,
    to_cwd: &str,
) -> Result<(usize, usize), MoveSessionError> {
    rewrite_history_jsonl_with_progress(path, session_id, from_cwd, to_cwd, &mut |_, _| {})
}

/// Rewrite `project` fields in `history.jsonl` for lines whose
/// `sessionId` matches `session_id` AND whose `project` matches
/// `from_cwd`. Returns `(rewritten, unmapped)` where `unmapped` is lines
/// whose `project` matches `from_cwd` but which lack a `sessionId`
/// field — typically pre-sessionId CC writes. Those are left alone
/// (we cannot attribute them to a single session).
///
/// Byte-exact for non-target lines.
pub(crate) fn rewrite_history_jsonl_with_progress(
    path: &Path,
    session_id: Uuid,
    from_cwd: &str,
    to_cwd: &str,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, usize), MoveSessionError> {
    let contents = fs::read_to_string(path)?;
    let total = contents.lines().count();
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("history.jsonl has no parent"))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;

    let old_kv = format!(r#""project":{}"#, encode_json_string(from_cwd)?);
    let new_kv = format!(r#""project":{}"#, encode_json_string(to_cwd)?);
    let sid_str = session_id.to_string();

    let mut rewritten = 0usize;
    let mut unmapped = 0usize;
    let mut done = 0usize;

    for line in contents.lines() {
        process_history_line(
            line,
            from_cwd,
            to_cwd,
            &sid_str,
            &old_kv,
            &new_kv,
            &mut tmp,
            &mut rewritten,
            &mut unmapped,
        )?;
        done += 1;
        on_progress(done, total);
    }
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok((rewritten, unmapped))
}

/// Per-line worker for [`rewrite_history_jsonl_with_progress`]. Pulled
/// out so the caller's loop can fire one `on_progress` per iteration
/// without touching the original early-return / fallback shape.
#[allow(clippy::too_many_arguments)]
fn process_history_line(
    line: &str,
    from_cwd: &str,
    to_cwd: &str,
    sid_str: &str,
    old_kv: &str,
    new_kv: &str,
    tmp: &mut tempfile::NamedTempFile,
    rewritten: &mut usize,
    unmapped: &mut usize,
) -> Result<(), MoveSessionError> {
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            writeln!(tmp, "{line}")?;
            return Ok(());
        }
    };
    let Some(obj) = parsed.as_object() else {
        writeln!(tmp, "{line}")?;
        return Ok(());
    };
    let project_matches = obj.get("project").and_then(|v| v.as_str()) == Some(from_cwd);
    let sid_matches = obj.get("sessionId").and_then(|v| v.as_str()) == Some(sid_str);

    if project_matches && sid_matches {
        if let Some(idx) = line.find(old_kv) {
            let mut out = String::with_capacity(line.len() + new_kv.len());
            out.push_str(&line[..idx]);
            out.push_str(new_kv);
            out.push_str(&line[idx + old_kv.len()..]);
            writeln!(tmp, "{out}")?;
            *rewritten += 1;
            return Ok(());
        }
        let old_spaced = format!(r#""project": {}"#, encode_json_string(from_cwd)?);
        let new_spaced = format!(r#""project": {}"#, encode_json_string(to_cwd)?);
        if let Some(idx) = line.find(&old_spaced) {
            let mut out = String::with_capacity(line.len() + new_spaced.len());
            out.push_str(&line[..idx]);
            out.push_str(&new_spaced);
            out.push_str(&line[idx + old_spaced.len()..]);
            writeln!(tmp, "{out}")?;
            *rewritten += 1;
            return Ok(());
        }
        writeln!(tmp, "{line}")?;
    } else if project_matches && !obj.contains_key("sessionId") {
        *unmapped += 1;
        writeln!(tmp, "{line}")?;
    } else {
        writeln!(tmp, "{line}")?;
    }
    Ok(())
}
