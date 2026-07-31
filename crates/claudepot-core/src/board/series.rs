//! Typed series: columns, values, and writer provenance.
//!
//! A series is an append-only typed table. Column types are fixed at
//! series creation — a push with a mismatched type is an **error, not a
//! coercion**. That rule is why the store can render a column without
//! re-sniffing every row, and why a bug in a writer surfaces at the
//! push rather than three weeks later as an unreadable chart.
//!
//! Nulls are legal in every column and mean *gap*, never zero. The
//! distinction is load-bearing: a monitoring board that renders a
//! missing sample as `0` reports an outage that did not happen.

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

use super::error::BoardError;

/// Maximum length of a board, series, column, or widget name.
pub const MAX_NAME_LEN: usize = 64;

/// Names are conservative on purpose: they reach SQLite, JSON, CSV
/// filenames, and the terminal. `[A-Za-z0-9._-]` is the intersection
/// that needs no escaping in any of those.
///
/// Rejects a leading `.` so a series can never produce a hidden or
/// `..`-shaped CSV filename during export.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if name.starts_with('.') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Validate a name, returning the canonical [`BoardError`] on failure.
///
/// The rejected name is redacted before it reaches the error string —
/// it is untrusted free text at this point, so it can be a token, a
/// terminal escape, or a megabyte.
pub fn check_name(name: &str) -> Result<(), BoardError> {
    if is_valid_name(name) {
        Ok(())
    } else {
        Err(BoardError::InvalidName(super::error::redact_identifier(
            name,
        )))
    }
}

/// The closed set of column types. Deliberately small — boards display
/// agent output, they do not model data (plan §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    /// RFC 3339 with an explicit offset. Stored normalized to UTC.
    Timestamp,
    /// f64. Non-finite values are rejected at ingest — NaN in a chart
    /// axis is a render bug with no useful meaning.
    Number,
    Integer,
    String,
    Bool,
}

impl ColumnType {
    pub fn as_str(self) -> &'static str {
        match self {
            ColumnType::Timestamp => "timestamp",
            ColumnType::Number => "number",
            ColumnType::Integer => "integer",
            ColumnType::String => "string",
            ColumnType::Bool => "bool",
        }
    }
}

/// One column in a series definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ColumnType,
}

impl Column {
    pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

/// A series definition — its name and fixed column layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesDef {
    pub name: String,
    pub columns: Vec<Column>,
}

impl SeriesDef {
    pub fn new(name: impl Into<String>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into(),
            columns,
        }
    }

    /// Structural validation, independent of any stored rows.
    pub fn validate(&self, max_columns: usize) -> Result<(), BoardError> {
        check_name(&self.name)?;
        if self.columns.is_empty() {
            return Err(BoardError::InvalidSpec(format!(
                "series `{}` has no columns",
                super::error::redact_identifier(&self.name)
            )));
        }
        if self.columns.len() > max_columns {
            return Err(BoardError::CapExceeded {
                what: "columns per series",
                limit: max_columns,
                actual: self.columns.len(),
            });
        }
        for col in &self.columns {
            check_name(&col.name)?;
        }
        let mut seen: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        if seen.len() != before {
            return Err(BoardError::InvalidSpec(format!(
                "series `{}` has duplicate column names",
                super::error::redact_identifier(&self.name)
            )));
        }
        Ok(())
    }
}

/// One cell. `Null` is legal in every column and renders as a gap.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Timestamp(DateTime<Utc>),
    Number(f64),
    Integer(i64),
    String(String),
    Bool(bool),
}

impl Value {
    /// Serialize for storage and export. Timestamps become RFC 3339 in
    /// UTC so the on-disk form is unambiguous and sorts lexically.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Timestamp(t) => serde_json::Value::String(t.to_rfc3339()),
            Value::Number(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                // Unreachable: non-finite is rejected at ingest. Null
                // is the honest fallback if it ever is reached — a gap
                // beats a fabricated number.
                .unwrap_or(serde_json::Value::Null),
            Value::Integer(i) => serde_json::Value::Number((*i).into()),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::Bool(b) => serde_json::Value::Bool(*b),
        }
    }

    /// Parse one cell against its declared column type.
    ///
    /// The column type is required rather than inferred, because JSON
    /// cannot distinguish a timestamp from a string, and an integer
    /// column receiving `3.0` should be an error rather than a silent
    /// truncation.
    pub fn from_json(
        raw: &serde_json::Value,
        column: &Column,
        series: &str,
    ) -> Result<Self, BoardError> {
        let mismatch = |actual: &'static str| BoardError::ColumnTypeMismatch {
            series: super::error::redact_identifier(series),
            column: super::error::redact_identifier(&column.name),
            expected: column.ty.as_str(),
            actual,
        };

        if raw.is_null() {
            return Ok(Value::Null);
        }

        match column.ty {
            ColumnType::Timestamp => {
                let s = raw.as_str().ok_or_else(|| mismatch(json_kind(raw)))?;
                let parsed = DateTime::<FixedOffset>::parse_from_rfc3339(s)
                    .map_err(|_| BoardError::InvalidTimestamp)?;
                Ok(Value::Timestamp(parsed.with_timezone(&Utc)))
            }
            ColumnType::Number => {
                let n = raw.as_f64().ok_or_else(|| mismatch(json_kind(raw)))?;
                if !n.is_finite() {
                    return Err(mismatch("non-finite number"));
                }
                Ok(Value::Number(n))
            }
            ColumnType::Integer => {
                let i = raw.as_i64().ok_or_else(|| mismatch(json_kind(raw)))?;
                Ok(Value::Integer(i))
            }
            ColumnType::String => {
                let s = raw.as_str().ok_or_else(|| mismatch(json_kind(raw)))?;
                Ok(Value::String(s.to_string()))
            }
            ColumnType::Bool => {
                let b = raw.as_bool().ok_or_else(|| mismatch(json_kind(raw)))?;
                Ok(Value::Bool(b))
            }
        }
    }

    /// Rendering for a **human-facing** surface: the terminal, the
    /// widget table, the GUI.
    ///
    /// Secret-shaped strings are redacted here. A board holds arbitrary
    /// agent output, and `rules/design.md` is absolute that credentials
    /// are never rendered — an agent that pushes a token into a cell
    /// must not have it painted into the window.
    ///
    /// Export deliberately does NOT use this; see [`to_export`].
    pub fn to_display(&self) -> String {
        match self {
            Value::String(s) => crate::session_live::redact::redact_secrets(s),
            other => other.to_export(),
        }
    }

    /// Rendering for **export**, where fidelity is the contract.
    ///
    /// A CSV or JSON export is the user's data leaving the app on their
    /// instruction. Redacting here would silently corrupt a round trip
    /// — the same reason `AGENTS.md` refuses to mask real data for
    /// screenshots.
    pub fn to_export(&self) -> String {
        match self {
            // A gap, rendered as a gap. Never `0`, never `false`.
            Value::Null => String::from("—"),
            Value::Timestamp(t) => t.to_rfc3339(),
            Value::Number(n) => format!("{n}"),
            Value::Integer(i) => i.to_string(),
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
        }
    }
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Which family of process claimed to write a row.
///
/// A **format** guarantee, not an identity one — see [`Provenance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterKind {
    AgentRun,
    CcSession,
    Cli,
    Import,
    System,
}

impl WriterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WriterKind::AgentRun => "agent_run",
            WriterKind::CcSession => "cc_session",
            WriterKind::Cli => "cli",
            WriterKind::Import => "import",
            WriterKind::System => "system",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent_run" => Some(WriterKind::AgentRun),
            "cc_session" => Some(WriterKind::CcSession),
            "cli" => Some(WriterKind::Cli),
            "import" => Some(WriterKind::Import),
            "system" => Some(WriterKind::System),
            _ => None,
        }
    }

    /// Human label for the UI and the terminal.
    pub fn label(self) -> &'static str {
        match self {
            WriterKind::AgentRun => "Agent run",
            WriterKind::CcSession => "Claude Code session",
            WriterKind::Cli => "CLI",
            WriterKind::Import => "Import",
            WriterKind::System => "System",
        }
    }
}

/// A writer's self-declared identity.
///
/// # This is not authenticated
///
/// There is no write channel and no credential (see the module docs on
/// [`super`]). Any local process that can open `boards.db` can
/// construct any `WriterId` it likes. Core records the claim; it cannot
/// verify it.
///
/// Every rendering surface must therefore say *reported*, never
/// *verified*. [`Provenance::reported_label`] produces the only
/// sanctioned phrasing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterId {
    pub kind: WriterKind,
    /// Free-text label, e.g. an agent name. Displayed, never trusted.
    pub label: String,
}

impl WriterId {
    pub fn new(kind: WriterKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
        }
    }

    /// Stable key for per-writer sequence tracking. Two writers sharing
    /// a key share a sequence space — which is a reason to make labels
    /// distinct, not a correctness problem.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.label)
    }
}

/// A row's recorded origin. Always a claim, never a fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub writer: WriterId,
    pub pushed_at: DateTime<Utc>,
    /// Always `false` in this design. Present so a future authenticated
    /// path can set it without a schema migration, and so no surface
    /// can render a "verified" badge by accident today.
    #[serde(default)]
    pub verified: bool,
}

impl Provenance {
    /// The **only** sanctioned phrasing for displaying a writer.
    ///
    /// Renders `Reported by: Nightly usage agent` rather than
    /// `Nightly usage agent wrote this`. Per `rules/design.md`, a
    /// status surface that shows an unverified claim as fact is a High
    /// finding — this method exists so that framing is impossible to
    /// forget at a call site.
    pub fn reported_label(&self) -> String {
        if self.verified {
            format!("Verified: {}", self.writer.label)
        } else {
            format!("Reported by: {}", self.writer.label)
        }
    }
}

/// One stored row: its values plus who claimed to write it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub values: Vec<Value>,
    pub provenance: Provenance,
    /// Per-writer monotonic sequence. Gaps are surfaced, not hidden.
    pub writer_seq: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: ColumnType) -> Column {
        Column::new(name, ty)
    }

    #[test]
    fn valid_names_accept_the_documented_charset() {
        assert!(is_valid_name("nightly-usage"));
        assert!(is_valid_name("cost_by_model"));
        assert!(is_valid_name("v1.2"));
        assert!(is_valid_name("a"));
    }

    #[test]
    fn invalid_names_are_rejected() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("has space"));
        assert!(!is_valid_name("slash/path"));
        assert!(!is_valid_name("back\\slash"));
        assert!(!is_valid_name("uni\u{00e9}"));
        assert!(!is_valid_name(&"x".repeat(MAX_NAME_LEN + 1)));
    }

    #[test]
    fn leading_dot_rejected_so_export_cannot_write_a_hidden_or_traversing_file() {
        assert!(!is_valid_name(".hidden"));
        assert!(!is_valid_name(".."));
        assert!(!is_valid_name("."));
    }

    #[test]
    fn null_is_accepted_in_every_column_type() {
        let raw = serde_json::Value::Null;
        for ty in [
            ColumnType::Timestamp,
            ColumnType::Number,
            ColumnType::Integer,
            ColumnType::String,
            ColumnType::Bool,
        ] {
            let v = Value::from_json(&raw, &col("c", ty), "s").unwrap();
            assert_eq!(v, Value::Null, "null rejected for {}", ty.as_str());
        }
    }

    #[test]
    fn a_secret_shaped_cell_is_redacted_on_display_but_survives_export() {
        // design.md is absolute that credentials are never rendered;
        // an export is the user's own data leaving on their
        // instruction, and redacting it would corrupt a round trip.
        let v = Value::String("sk-ant-oat01-AbcDefGhiJkl".to_string());
        assert!(
            !v.to_display().contains("AbcDefGhiJkl"),
            "leaked: {}",
            v.to_display()
        );
        assert!(
            v.to_export().contains("AbcDefGhiJkl"),
            "export lost fidelity"
        );
    }

    #[test]
    fn ordinary_text_is_untouched_by_the_display_path() {
        let v = Value::String("claude-opus-5".to_string());
        assert_eq!(v.to_display(), "claude-opus-5");
        assert_eq!(v.to_export(), "claude-opus-5");
    }

    #[test]
    fn unknown_keys_are_rejected_in_a_series_definition() {
        // plan §7: unknown keys rejected, not ignored.
        let raw = r#"{"name":"s","columns":[{"name":"c","type":"integer"}],"renderer":{"raw":1}}"#;
        assert!(serde_json::from_str::<SeriesDef>(raw).is_err());
    }

    #[test]
    fn null_renders_as_a_gap_not_zero() {
        // A monitoring board that shows a missing sample as 0 reports
        // an outage that did not happen.
        assert_eq!(Value::Null.to_display(), "—");
        assert_ne!(Value::Null.to_display(), "0");
        assert_eq!(Value::Null.to_json(), serde_json::Value::Null);
    }

    #[test]
    fn timestamps_require_an_explicit_offset() {
        let c = col("t", ColumnType::Timestamp);
        let ok = serde_json::json!("2026-07-31T09:14:03+02:00");
        assert!(Value::from_json(&ok, &c, "s").is_ok());

        let naive = serde_json::json!("2026-07-31T09:14:03");
        assert!(matches!(
            Value::from_json(&naive, &c, "s"),
            Err(BoardError::InvalidTimestamp)
        ));
    }

    #[test]
    fn timestamps_normalize_to_utc() {
        let c = col("t", ColumnType::Timestamp);
        let raw = serde_json::json!("2026-07-31T09:00:00+02:00");
        let v = Value::from_json(&raw, &c, "s").unwrap();
        match v {
            Value::Timestamp(t) => assert_eq!(t.to_rfc3339(), "2026-07-31T07:00:00+00:00"),
            other => panic!("expected timestamp, got {other:?}"),
        }
    }

    #[test]
    fn integer_column_rejects_a_float() {
        let c = col("n", ColumnType::Integer);
        let raw = serde_json::json!(3.5);
        assert!(matches!(
            Value::from_json(&raw, &c, "s"),
            Err(BoardError::ColumnTypeMismatch { .. })
        ));
    }

    #[test]
    fn number_column_rejects_a_string_rather_than_coercing() {
        let c = col("n", ColumnType::Number);
        let raw = serde_json::json!("12");
        assert!(matches!(
            Value::from_json(&raw, &c, "s"),
            Err(BoardError::ColumnTypeMismatch { .. })
        ));
    }

    #[test]
    fn json_cannot_smuggle_a_non_finite_number_into_a_series() {
        // NaN or Infinity on a chart axis has no useful rendering. The
        // guard in `from_json` is defense in depth for a future
        // non-JSON input path; the *actual* barrier is that
        // `serde_json::Number` cannot hold a non-finite value at all,
        // so no text reaches `from_json` carrying one. Assert the
        // barrier, since that is the property being relied on.
        for literal in ["1e400", "-1e400", "NaN", "Infinity", "-Infinity"] {
            assert!(
                serde_json::from_str::<serde_json::Value>(literal).is_err(),
                "serde_json accepted `{literal}`, so the from_json guard is now load-bearing"
            );
        }
    }

    #[test]
    fn string_column_rejects_an_object() {
        let c = col("s", ColumnType::String);
        let raw = serde_json::json!({"nested": true});
        assert!(matches!(
            Value::from_json(&raw, &c, "s"),
            Err(BoardError::ColumnTypeMismatch {
                actual: "object",
                ..
            })
        ));
    }

    #[test]
    fn series_validate_rejects_duplicate_columns() {
        let s = SeriesDef::new(
            "dupes",
            vec![col("a", ColumnType::Integer), col("a", ColumnType::String)],
        );
        assert!(matches!(s.validate(16), Err(BoardError::InvalidSpec(_))));
    }

    #[test]
    fn series_validate_rejects_empty_columns() {
        let s = SeriesDef::new("empty", vec![]);
        assert!(matches!(s.validate(16), Err(BoardError::InvalidSpec(_))));
    }

    #[test]
    fn series_validate_enforces_the_column_cap() {
        let cols = (0..20)
            .map(|i| col(&format!("c{i}"), ColumnType::Integer))
            .collect();
        let s = SeriesDef::new("wide", cols);
        assert!(matches!(
            s.validate(16),
            Err(BoardError::CapExceeded { limit: 16, .. })
        ));
    }

    #[test]
    fn provenance_never_claims_verification_by_default() {
        let p = Provenance {
            writer: WriterId::new(WriterKind::AgentRun, "Nightly usage agent"),
            pushed_at: Utc::now(),
            verified: false,
        };
        assert_eq!(p.reported_label(), "Reported by: Nightly usage agent");
        assert!(!p.reported_label().contains("Verified"));
    }

    #[test]
    fn writer_kind_round_trips_through_its_wire_string() {
        for k in [
            WriterKind::AgentRun,
            WriterKind::CcSession,
            WriterKind::Cli,
            WriterKind::Import,
            WriterKind::System,
        ] {
            assert_eq!(WriterKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(WriterKind::parse("nope"), None);
    }
}
