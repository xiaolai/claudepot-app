//! Error type for the boards store.
//!
//! One enum per module boundary, per `rules/rust-conventions.md`.
//!
//! # Nothing here echoes an untrusted string verbatim
//!
//! Two separate rules, and the second was missed on the first pass:
//!
//! 1. **Row values never appear.** A board holds arbitrary agent
//!    output and an error string reaches logs, so errors name the
//!    column and the expected type, never the offending cell.
//! 2. **Identifiers are redacted too.** A board name, series name, or
//!    writer label is also user-controlled. `sk-ant-oat01-Abc` is a
//!    *valid* series name under the `[A-Za-z0-9._-]` charset, so an
//!    error that echoed a name verbatim would print a token — exactly
//!    what `rules/rust-conventions.md` forbids. Every constructor
//!    below routes its identifier through [`redact_identifier`].

/// Longest identifier fragment an error will echo.
const MAX_ECHO: usize = 32;

/// Make a user-controlled identifier safe to put in an error string.
///
/// Masks anything token-shaped outright, bounds the length, and strips
/// control characters (which can rewrite a terminal line and forge the
/// rest of a log entry).
///
/// Use this at every `BoardError` construction site that carries a
/// name, series, or writer label.
pub fn redact_identifier(raw: &str) -> String {
    // `rules/rust-conventions.md`: never print a string starting with
    // `sk-ant-`. Checked case-insensitively and after trimming, since
    // the point is to catch a token however it was pasted.
    let probe = raw.trim().to_ascii_lowercase();
    if probe.starts_with("sk-ant-") {
        return "sk-ant-<redacted>".to_string();
    }

    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .take(MAX_ECHO)
        .collect();
    if raw.chars().count() > MAX_ECHO {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

/// Failures from the SQLite-backed boards store and its write path.
#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("no board with id `{0}`")]
    BoardNotFound(String),

    #[error("board `{board}` has no series named `{series}`")]
    SeriesNotFound { board: String, series: String },

    #[error("board `{board}` already has a series named `{series}`")]
    DuplicateSeries { board: String, series: String },

    /// Revision-checked mutation lost a race. Never last-write-wins
    /// (plan F8) — the caller re-reads and retries.
    #[error("board `{board}` is at revision {current}, not {base}; re-read and retry")]
    RevisionConflict {
        board: String,
        base: i64,
        current: i64,
    },

    #[error("series `{series}` column `{column}` expects {expected}, got {actual}")]
    ColumnTypeMismatch {
        series: String,
        column: String,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("series `{series}` expects {expected} columns per row, got {actual}")]
    ColumnCountMismatch {
        series: String,
        expected: usize,
        actual: usize,
    },

    #[error("`{0}` is not a valid name: use 1-64 chars of [A-Za-z0-9._-]")]
    InvalidName(String),

    #[error("timestamp is not RFC 3339 with an explicit offset")]
    InvalidTimestamp,

    /// A writer replayed a sequence number it already used with
    /// different data. Distinct from an idempotent retry, which is a
    /// silent no-op.
    #[error("writer `{writer}` reused sequence {seq} on series `{series}` with different rows")]
    SequenceReplay {
        writer: String,
        series: String,
        seq: i64,
    },

    #[error("{what} exceeds the cap of {limit} ({actual})")]
    CapExceeded {
        what: &'static str,
        limit: usize,
        actual: usize,
    },

    #[error("board spec is invalid: {0}")]
    InvalidSpec(String),

    #[error("export envelope has schema version {found}, expected {expected}")]
    UnsupportedExportVersion { found: u32, expected: u32 },

    #[error("boards.db is at schema version {found}, newer than this build supports ({supported}); upgrade Claudepot")]
    SchemaTooNew { found: i64, supported: i64 },

    /// A writer sequence that cannot be stored: below 1, or a range
    /// that would overflow `i64`. Rejected rather than wrapped —
    /// a wrapped sequence silently reorders history.
    #[error("writer sequence {seq} is out of range for a {rows}-row push (must be >= 1 and not overflow)")]
    SequenceOutOfRange { seq: i64, rows: usize },

    /// An export envelope could not be parsed.
    ///
    /// Deliberately carries no detail from `serde_json`. Serde's
    /// "unknown variant `X`" message echoes the raw value, and an
    /// envelope is untrusted input — a token-shaped `ColumnType` or
    /// `WidgetKind` would print verbatim, straight past
    /// [`redact_identifier`], which only guards errors we construct
    /// ourselves.
    #[error("export envelope is not valid: {0}")]
    InvalidEnvelope(&'static str),

    /// A stored row could not be read back. Within a supported schema
    /// version this is corruption, so it fails loud rather than
    /// rendering a plausible substitute.
    #[error("board row in series `{series}` is corrupt: {what}")]
    CorruptRow { series: String, what: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_shaped_identifier_is_masked() {
        // A series name is validated as [A-Za-z0-9._-], which a real
        // token satisfies — so name validation is not a defense here.
        assert_eq!(
            redact_identifier("sk-ant-oat01-AbcDefGhi"),
            "sk-ant-<redacted>"
        );
        assert_eq!(redact_identifier("  SK-ANT-oat01-x  "), "sk-ant-<redacted>");
    }

    #[test]
    fn an_ordinary_identifier_survives_intact() {
        assert_eq!(redact_identifier("nightly-usage"), "nightly-usage");
        assert_eq!(redact_identifier(""), "");
    }

    #[test]
    fn long_identifiers_are_bounded() {
        let long = "a".repeat(100);
        let out = redact_identifier(&long);
        assert!(out.chars().count() <= MAX_ECHO + 1, "unbounded: {out}");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn control_characters_cannot_forge_a_log_line() {
        let sneaky = "ok\r\n2026-01-01 INFO all clear";
        let out = redact_identifier(sneaky);
        assert!(!out.contains('\n'), "newline survived: {out:?}");
        assert!(!out.contains('\r'), "carriage return survived: {out:?}");
    }

    #[test]
    fn error_display_does_not_leak_a_token_bearing_name() {
        let err = BoardError::InvalidName(redact_identifier("sk-ant-oat01-SECRET"));
        let text = err.to_string();
        assert!(!text.contains("SECRET"), "leaked: {text}");
    }
}
