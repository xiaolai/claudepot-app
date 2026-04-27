//! Environment-variable whitelist for automation runs.
//!
//! launchd, Task Scheduler, and systemd-user all run with a
//! stripped environment by default. We materialize a curated set
//! of variables on every scheduled run, plus any user-supplied
//! `extra_env` entries that pass the whitelist.
//!
//! Forbidden keys: `ANTHROPIC_*`, `CLAUDE_*` (route wrappers and
//! the first-party `claude` binary set those themselves — letting
//! a user override them via this surface is a footgun and a
//! credential-leak risk), and the four "we set this" basics:
//! `PATH`, `HOME`, `LANG`, `LC_ALL`, `TERM`.
//!
//! Allowed key shape: ASCII alnum + underscore, no leading digit.
//! Values: any printable ASCII (we reject newlines + NUL so the
//! plist/XML/unit-file emitters don't have to escape multiline
//! input).

use std::collections::BTreeMap;

use super::error::AutomationError;

/// Keys we set ourselves on every run; user can't override.
const RESERVED_EXACT: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TERM"];

/// Prefixes the user can never override — those belong to the
/// binary picker (first-party slot or route wrapper).
const RESERVED_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_"];

/// Validate a single user-supplied env entry. Returns `Ok(())` if
/// the entry is allowed; an [`AutomationError::InvalidEnv`]
/// describing the reason otherwise.
pub fn validate_entry(key: &str, value: &str) -> Result<(), AutomationError> {
    validate_key(key)?;
    validate_value(value)?;
    Ok(())
}

/// Validate every entry in the user's `extra_env`. Returns the map
/// unchanged on success.
pub fn validate_map(map: &BTreeMap<String, String>) -> Result<(), AutomationError> {
    for (k, v) in map {
        validate_entry(k, v)?;
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), AutomationError> {
    if key.is_empty() {
        return Err(AutomationError::InvalidEnv("empty key".into()));
    }
    if key.len() > 256 {
        return Err(AutomationError::InvalidEnv(format!(
            "key '{key}' exceeds 256 characters"
        )));
    }
    let bytes = key.as_bytes();
    if !matches!(bytes[0], b'A'..=b'Z' | b'a'..=b'z' | b'_') {
        return Err(AutomationError::InvalidEnv(format!(
            "key '{key}' must start with a letter or underscore"
        )));
    }
    if !bytes.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(AutomationError::InvalidEnv(format!(
            "key '{key}' must be ASCII alnum + underscore only"
        )));
    }
    if RESERVED_EXACT.iter().any(|r| r.eq_ignore_ascii_case(key)) {
        return Err(AutomationError::InvalidEnv(format!(
            "key '{key}' is set by Claudepot and cannot be overridden"
        )));
    }
    if RESERVED_PREFIXES
        .iter()
        .any(|p| key.to_ascii_uppercase().starts_with(p))
    {
        return Err(AutomationError::InvalidEnv(format!(
            "key '{key}' is reserved (ANTHROPIC_*/CLAUDE_* belong to the binary picker)"
        )));
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<(), AutomationError> {
    if value.len() > 4096 {
        return Err(AutomationError::InvalidEnv(format!(
            "value exceeds 4096 characters ({})",
            value.len()
        )));
    }
    for &b in value.as_bytes() {
        if b == b'\n' || b == b'\r' || b == 0 {
            return Err(AutomationError::InvalidEnv(
                "value contains newline or NUL".into(),
            ));
        }
        if !(b == b'\t' || (0x20..=0x7e).contains(&b)) {
            return Err(AutomationError::InvalidEnv(
                "value must be printable ASCII".into(),
            ));
        }
    }
    Ok(())
}

/// Default `PATH` segments to materialize on every run, in order.
/// Adapters concatenate these with the platform separator (`:` on
/// unix, `;` on Windows). Caller-supplied `claudepot_bin_dir` is
/// always last so route wrappers resolve.
pub fn default_path_segments(claudepot_bin_dir: &str) -> Vec<String> {
    let mut v: Vec<String> = if cfg!(target_os = "windows") {
        // Windows: trust the inherited PATH less; system32 is
        // assumed to already be there via the user's profile, but
        // the scheduler may strip it. We add the typical user
        // shimmed locations.
        vec![
            r"%SystemRoot%\System32".to_string(),
            r"%SystemRoot%".to_string(),
            r"%SystemRoot%\System32\Wbem".to_string(),
            r"%USERPROFILE%\.local\bin".to_string(),
        ]
    } else {
        vec![
            "/usr/local/bin".to_string(),
            "/opt/homebrew/bin".to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
            // Common per-user shim locations.
            format!(
                "{}/.local/bin",
                std::env::var("HOME").unwrap_or_default()
            ),
        ]
    };
    if !claudepot_bin_dir.is_empty() {
        v.push(claudepot_bin_dir.to_string());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn accepts_simple_user_env() {
        validate_entry("FOO", "bar").unwrap();
        validate_entry("MY_VAR_2", "value with spaces").unwrap();
        validate_entry("_PRIVATE", "ok").unwrap();
        validate_map(&map(&[("A", "1"), ("B", "2")])).unwrap();
    }

    #[test]
    fn rejects_anthropic_and_claude_prefixes() {
        assert!(validate_entry("ANTHROPIC_API_KEY", "sk-...").is_err());
        assert!(validate_entry("anthropic_base_url", "x").is_err());
        assert!(validate_entry("CLAUDE_CODE_SOMETHING", "x").is_err());
    }

    #[test]
    fn rejects_path_home_and_friends() {
        for k in ["PATH", "path", "HOME", "Home", "LANG", "LC_ALL", "TERM"] {
            assert!(validate_entry(k, "x").is_err(), "should reject {k}");
        }
    }

    #[test]
    fn rejects_empty_key() {
        assert!(validate_entry("", "x").is_err());
    }

    #[test]
    fn rejects_leading_digit() {
        assert!(validate_entry("1FOO", "x").is_err());
    }

    #[test]
    fn rejects_punct_in_key() {
        assert!(validate_entry("FOO-BAR", "x").is_err());
        assert!(validate_entry("FOO.BAR", "x").is_err());
        assert!(validate_entry("FOO BAR", "x").is_err());
    }

    #[test]
    fn rejects_newline_in_value() {
        assert!(validate_entry("FOO", "line1\nline2").is_err());
        assert!(validate_entry("FOO", "line1\rmore").is_err());
    }

    #[test]
    fn rejects_nul_in_value() {
        assert!(validate_entry("FOO", "a\0b").is_err());
    }

    #[test]
    fn rejects_overlong() {
        assert!(validate_entry(&"A".repeat(257), "x").is_err());
        assert!(validate_entry("FOO", &"a".repeat(4097)).is_err());
    }

    #[test]
    fn default_path_segments_includes_claudepot_bin() {
        let segs = default_path_segments("/Users/x/.claudepot/bin");
        assert!(segs.last().unwrap() == "/Users/x/.claudepot/bin");
        assert!(!segs.is_empty());
    }
}
