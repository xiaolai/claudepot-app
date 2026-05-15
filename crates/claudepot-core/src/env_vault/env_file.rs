//! Line-oriented `.env` file editor.
//!
//! Every mutation touches **only the target key's line**. Untouched
//! lines are preserved byte-for-byte — blank lines, ordering, header
//! comments, and comments on other lines all survive. The file is
//! never round-tripped through a parsed-then-re-serialized model,
//! which is the failure mode that silently drops structure.
//!
//! One documented exception: [`set`] rewrites the *entire* target
//! line, so an inline trailing comment on that one line (e.g.
//! `KEY=v # note`) is dropped. Every other line is untouched, and
//! `comment` / `uncomment` / `delete` preserve inline comments fully
//! because they toggle or remove the whole line.
//!
//! Scope: this is a *movement* layer, not a `.env` parser. It does
//! not expand `${VAR}` references, unquote values, or evaluate
//! last-wins precedence. Mutations act on the *first* line matching
//! the key.

/// One classified line of a `.env` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvLine {
    /// An active assignment — `KEY=value` (optionally `export KEY=…`).
    Active { key: String, value: String },
    /// A commented-out assignment — `# KEY=value`, `#KEY=value`, etc.
    /// The value is still on disk; the key is just inactive.
    Commented { key: String, value: String },
    /// Anything else: blank lines, prose comments, malformed lines.
    Other(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvEditError {
    #[error("no `.env` line found for key `{0}`")]
    KeyNotFound(String),
    #[error("key `{0}` is already active")]
    AlreadyActive(String),
    #[error("key `{0}` is already commented out")]
    AlreadyCommented(String),
    #[error("`{0}` is not a valid env key name")]
    InvalidKeyName(String),
    #[error("value for `{0}` contains a newline, which `.env` cannot represent")]
    ValueHasNewline(String),
}

/// True for a syntactically valid env key: `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Parse `KEY=value` (with an optional leading `export `) out of a
/// trimmed-start string. Returns `(key, value)` — the value is
/// everything after the first `=`, verbatim.
fn parse_assignment(s: &str) -> Option<(String, String)> {
    let body = s.strip_prefix("export ").map(str::trim_start).unwrap_or(s);
    let eq = body.find('=')?;
    let key = body[..eq].trim_end();
    if !is_valid_key(key) {
        return None;
    }
    Some((key.to_string(), body[eq + 1..].to_string()))
}

/// Classify a single raw line (without its trailing newline).
fn classify(line: &str) -> EnvLine {
    let stripped = line.trim_start();
    if let Some((key, value)) = parse_assignment(stripped) {
        return EnvLine::Active { key, value };
    }
    if let Some(rest) = stripped.strip_prefix('#') {
        if let Some((key, value)) = parse_assignment(rest.trim_start()) {
            return EnvLine::Commented { key, value };
        }
    }
    EnvLine::Other(line.to_string())
}

/// Parse a whole `.env` file's content into classified lines, in
/// file order. The terminal newline (if any) is not represented as a
/// line — `parse("A=1\n")` yields one entry.
pub fn parse(content: &str) -> Vec<EnvLine> {
    split_lines(content).iter().map(|l| classify(l)).collect()
}

/// Split content into logical lines, remembering whether the file
/// ended with a newline so [`join_lines`] can reproduce it.
fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let had_trailing_newline = content.ends_with('\n');
    let mut lines: Vec<String> = content.split('\n').map(str::to_string).collect();
    // `"A=1\n".split('\n')` → ["A=1", ""]; drop the synthetic empty
    // tail so a trailing newline isn't mistaken for a blank line.
    if had_trailing_newline {
        lines.pop();
    }
    lines
}

/// Rejoin lines, always terminating with a newline (the POSIX-correct
/// shape for a text file; also what every editor produces).
fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Index of the first line matching `key`, with a predicate over the
/// classified line. Used to target a specific state (active vs
/// commented) for a mutation.
fn find_line(
    lines: &[String],
    key: &str,
    want: impl Fn(&EnvLine) -> bool,
) -> Option<usize> {
    lines
        .iter()
        .position(|l| matches_key(&classify(l), key) && want(&classify(l)))
}

fn matches_key(line: &EnvLine, key: &str) -> bool {
    match line {
        EnvLine::Active { key: k, .. } | EnvLine::Commented { key: k, .. } => k == key,
        EnvLine::Other(_) => false,
    }
}

/// Upsert `key` to an active `KEY=value`. If the key already exists
/// (active *or* commented), its first line is replaced in place — the
/// key becomes active with the new value, preserving file position.
/// Otherwise a new `KEY=value` line is appended.
///
/// Rewrites the whole target line, so an inline trailing comment on
/// that specific line is dropped (see module docs). Errors on an
/// invalid key name or a value containing a newline.
pub fn set(content: &str, key: &str, value: &str) -> Result<String, EnvEditError> {
    if !is_valid_key(key) {
        return Err(EnvEditError::InvalidKeyName(key.to_string()));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(EnvEditError::ValueHasNewline(key.to_string()));
    }
    let mut lines = split_lines(content);
    let new_line = format!("{key}={value}");
    match find_line(&lines, key, |_| true) {
        Some(i) => lines[i] = new_line,
        None => lines.push(new_line),
    }
    Ok(join_lines(&lines))
}

/// Comment out the first *active* line for `key` (`KEY=value` →
/// `# KEY=value`). The value stays on disk; the key just goes
/// inactive. Errors if the key isn't found, or is already commented.
pub fn comment(content: &str, key: &str) -> Result<String, EnvEditError> {
    let mut lines = split_lines(content);
    if find_line(&lines, key, |l| matches!(l, EnvLine::Active { .. })).is_none() {
        // Distinguish "not here at all" from "already commented".
        return Err(
            if find_line(&lines, key, |l| matches!(l, EnvLine::Commented { .. })).is_some() {
                EnvEditError::AlreadyCommented(key.to_string())
            } else {
                EnvEditError::KeyNotFound(key.to_string())
            },
        );
    }
    let i = find_line(&lines, key, |l| matches!(l, EnvLine::Active { .. })).unwrap();
    lines[i] = format!("# {}", lines[i].trim_start());
    Ok(join_lines(&lines))
}

/// Uncomment the first *commented* line for `key` (`# KEY=value` →
/// `KEY=value`). Errors if the key isn't found, or is already active.
/// Leading whitespace and the `#` marker are stripped; an indented
/// commented line comes back un-indented.
pub fn uncomment(content: &str, key: &str) -> Result<String, EnvEditError> {
    let mut lines = split_lines(content);
    if find_line(&lines, key, |l| matches!(l, EnvLine::Commented { .. })).is_none() {
        return Err(
            if find_line(&lines, key, |l| matches!(l, EnvLine::Active { .. })).is_some() {
                EnvEditError::AlreadyActive(key.to_string())
            } else {
                EnvEditError::KeyNotFound(key.to_string())
            },
        );
    }
    let i = find_line(&lines, key, |l| matches!(l, EnvLine::Commented { .. })).unwrap();
    let body = lines[i]
        .trim_start()
        .strip_prefix('#')
        .map(str::trim_start)
        .unwrap_or(&lines[i])
        .to_string();
    lines[i] = body;
    Ok(join_lines(&lines))
}

/// Delete the first line for `key` — active or commented. Errors if
/// the key isn't found anywhere.
pub fn delete(content: &str, key: &str) -> Result<String, EnvEditError> {
    let mut lines = split_lines(content);
    match find_line(&lines, key, |_| true) {
        Some(i) => {
            lines.remove(i);
            Ok(join_lines(&lines))
        }
        None => Err(EnvEditError::KeyNotFound(key.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_key_accepts_and_rejects() {
        for ok in ["A", "_A", "API_KEY", "k0", "_"] {
            assert!(is_valid_key(ok), "{ok} should be valid");
        }
        for bad in ["", "0K", "A-B", "A B", "A.B", "FOO=", "ç"] {
            assert!(!is_valid_key(bad), "{bad} should be invalid");
        }
    }

    #[test]
    fn classify_handles_each_line_shape() {
        assert_eq!(
            classify("API_KEY=sk-123"),
            EnvLine::Active {
                key: "API_KEY".into(),
                value: "sk-123".into()
            }
        );
        assert_eq!(
            classify("export API_KEY=sk-123"),
            EnvLine::Active {
                key: "API_KEY".into(),
                value: "sk-123".into()
            }
        );
        assert_eq!(
            classify("# API_KEY=sk-123"),
            EnvLine::Commented {
                key: "API_KEY".into(),
                value: "sk-123".into()
            }
        );
        assert_eq!(
            classify("#API_KEY=sk-123"),
            EnvLine::Commented {
                key: "API_KEY".into(),
                value: "sk-123".into()
            }
        );
        assert_eq!(
            classify("# just a prose comment"),
            EnvLine::Other("# just a prose comment".into())
        );
        assert_eq!(classify(""), EnvLine::Other("".into()));
        assert_eq!(
            classify("not an assignment"),
            EnvLine::Other("not an assignment".into())
        );
    }

    #[test]
    fn value_with_equals_and_hash_is_kept_verbatim() {
        assert_eq!(
            classify("URL=postgres://u:p@h/db?x=1#frag"),
            EnvLine::Active {
                key: "URL".into(),
                value: "postgres://u:p@h/db?x=1#frag".into()
            }
        );
    }

    #[test]
    fn parse_drops_synthetic_trailing_line() {
        assert_eq!(parse("A=1\n").len(), 1);
        assert_eq!(parse("A=1").len(), 1);
        // A genuine blank line in the middle is preserved.
        assert_eq!(parse("A=1\n\nB=2\n").len(), 3);
        assert!(parse("").is_empty());
    }

    const SAMPLE: &str = "# header comment\n\
                          API_KEY=sk-old\n\
                          \n\
                          # DB_URL=postgres://localhost\n\
                          PORT=3000 # inline note\n";

    #[test]
    fn set_replaces_existing_key_in_place_preserving_everything_else() {
        let out = set(SAMPLE, "API_KEY", "sk-new").unwrap();
        assert_eq!(
            out,
            "# header comment\n\
             API_KEY=sk-new\n\
             \n\
             # DB_URL=postgres://localhost\n\
             PORT=3000 # inline note\n"
        );
    }

    #[test]
    fn set_appends_a_new_key() {
        let out = set("A=1\n", "B", "2").unwrap();
        assert_eq!(out, "A=1\nB=2\n");
    }

    #[test]
    fn set_on_empty_content_creates_the_file_body() {
        assert_eq!(set("", "A", "1").unwrap(), "A=1\n");
    }

    #[test]
    fn set_promotes_a_commented_key_to_active() {
        let out = set("# A=old\n", "A", "new").unwrap();
        assert_eq!(out, "A=new\n");
    }

    #[test]
    fn set_rejects_invalid_key_and_newline_value() {
        assert_eq!(
            set("", "0BAD", "v"),
            Err(EnvEditError::InvalidKeyName("0BAD".into()))
        );
        assert_eq!(
            set("", "OK", "line1\nline2"),
            Err(EnvEditError::ValueHasNewline("OK".into()))
        );
    }

    #[test]
    fn comment_toggles_active_to_commented_leaving_rest_intact() {
        let out = comment(SAMPLE, "API_KEY").unwrap();
        assert_eq!(
            out,
            "# header comment\n\
             # API_KEY=sk-old\n\
             \n\
             # DB_URL=postgres://localhost\n\
             PORT=3000 # inline note\n"
        );
        // The newly-commented line classifies as Commented with the
        // value still recoverable.
        assert_eq!(
            classify("# API_KEY=sk-old"),
            EnvLine::Commented {
                key: "API_KEY".into(),
                value: "sk-old".into()
            }
        );
    }

    #[test]
    fn comment_errors_when_missing_or_already_commented() {
        assert_eq!(
            comment(SAMPLE, "NOPE"),
            Err(EnvEditError::KeyNotFound("NOPE".into()))
        );
        assert_eq!(
            comment(SAMPLE, "DB_URL"),
            Err(EnvEditError::AlreadyCommented("DB_URL".into()))
        );
    }

    #[test]
    fn uncomment_toggles_commented_to_active() {
        let out = uncomment(SAMPLE, "DB_URL").unwrap();
        assert_eq!(
            out,
            "# header comment\n\
             API_KEY=sk-old\n\
             \n\
             DB_URL=postgres://localhost\n\
             PORT=3000 # inline note\n"
        );
    }

    #[test]
    fn uncomment_errors_when_missing_or_already_active() {
        assert_eq!(
            uncomment(SAMPLE, "NOPE"),
            Err(EnvEditError::KeyNotFound("NOPE".into()))
        );
        assert_eq!(
            uncomment(SAMPLE, "API_KEY"),
            Err(EnvEditError::AlreadyActive("API_KEY".into()))
        );
    }

    #[test]
    fn comment_then_uncomment_round_trips() {
        let commented = comment(SAMPLE, "API_KEY").unwrap();
        let back = uncomment(&commented, "API_KEY").unwrap();
        assert_eq!(back, SAMPLE);
    }

    #[test]
    fn delete_removes_active_or_commented_line() {
        let without_active = delete(SAMPLE, "API_KEY").unwrap();
        assert_eq!(
            without_active,
            "# header comment\n\
             \n\
             # DB_URL=postgres://localhost\n\
             PORT=3000 # inline note\n"
        );
        let without_commented = delete(SAMPLE, "DB_URL").unwrap();
        assert_eq!(
            without_commented,
            "# header comment\n\
             API_KEY=sk-old\n\
             \n\
             PORT=3000 # inline note\n"
        );
    }

    #[test]
    fn delete_errors_when_key_absent() {
        assert_eq!(
            delete(SAMPLE, "NOPE"),
            Err(EnvEditError::KeyNotFound("NOPE".into()))
        );
    }

    #[test]
    fn mutations_normalize_a_missing_trailing_newline() {
        // Input lacks the final newline; output is POSIX-correct.
        assert_eq!(set("A=1", "A", "2").unwrap(), "A=2\n");
        assert_eq!(delete("A=1\nB=2", "B").unwrap(), "A=1\n");
    }

    #[test]
    fn parse_exposes_tri_state_for_the_ui() {
        let lines = parse(SAMPLE);
        let active: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                EnvLine::Active { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        let commented: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                EnvLine::Commented { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(active, vec!["API_KEY", "PORT"]);
        assert_eq!(commented, vec!["DB_URL"]);
    }
}
