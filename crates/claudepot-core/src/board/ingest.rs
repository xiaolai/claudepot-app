//! The write path: validation, caps, idempotency, and ordering.
//!
//! Everything here is pure. [`validate_push`] takes a request and a
//! series definition and returns typed rows or an error; it touches no
//! database and no clock. The transaction that consumes its output
//! lives in [`super::store::BoardStore::push`], which is also where
//! idempotency is *enforced* — checking a dedup key outside the
//! transaction that writes is a race by construction.

use super::error::BoardError;
use super::series::{SeriesDef, Value, WriterId};

/// Whether a push extends a series or replaces it wholesale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMode {
    Append,
    Replace,
}

impl PushMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "append" => Some(PushMode::Append),
            "replace" => Some(PushMode::Replace),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PushMode::Append => "append",
            PushMode::Replace => "replace",
        }
    }
}

/// Bounds on what one writer can put into a board.
///
/// These exist because a runaway agent must not be able to fill the
/// disk, and because a chart with ten million points is not a chart.
/// The numbers are a tuning decision; the *presence* of a named,
/// tested limit for each dimension is the design commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestCaps {
    pub max_rows_per_push: usize,
    pub max_rows_per_series: usize,
    pub max_series_per_board: usize,
    pub max_columns_per_series: usize,
    /// Cap on a single string cell, in bytes. Tool output is arbitrary
    /// stdout; an unbounded cell turns a board into a log sink.
    pub max_cell_bytes: usize,
}

impl Default for IngestCaps {
    fn default() -> Self {
        Self {
            max_rows_per_push: 10_000,
            max_rows_per_series: 1_000_000,
            max_series_per_board: 32,
            max_columns_per_series: 16,
            max_cell_bytes: 8 * 1024,
        }
    }
}

/// One push of rows onto one series.
#[derive(Debug, Clone)]
pub struct PushRequest {
    pub board_id: String,
    pub series: String,
    /// Each element is a JSON array of cells, positionally matching the
    /// series' column list.
    pub rows: Vec<serde_json::Value>,
    pub mode: PushMode,
    /// Self-reported. See [`super::series::WriterId`] — core records
    /// this claim and cannot verify it.
    pub writer: WriterId,
    /// Dedup key. A repeat is a no-op returning the original result.
    pub idem_key: Option<String>,
    /// Explicit starting sequence. Omit to continue after this writer's
    /// last row.
    pub writer_seq: Option<i64>,
}

/// What a push did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    pub rows_added: usize,
    /// True when an `idem_key` matched a previous push and nothing was
    /// written.
    pub deduplicated: bool,
    /// `(first_missing, last_missing)` when the writer's sequence
    /// jumped. Surfaced rather than hidden — a silent gap is a reader
    /// quietly rendering an incomplete series as complete.
    pub sequence_gap: Option<(i64, i64)>,
}

/// Type-check and bound one push against its series definition.
///
/// Returns rows in request order. Every failure is positional and names
/// the column, so a writer can find the offending field — but never
/// quotes the cell's value, because a board can hold arbitrary agent
/// output and an error string reaches logs.
pub fn validate_push(
    req: &PushRequest,
    def: &SeriesDef,
    caps: &IngestCaps,
) -> Result<Vec<Vec<Value>>, BoardError> {
    if req.rows.len() > caps.max_rows_per_push {
        return Err(BoardError::CapExceeded {
            what: "rows per push",
            limit: caps.max_rows_per_push,
            actual: req.rows.len(),
        });
    }

    let mut out = Vec::with_capacity(req.rows.len());
    for raw in &req.rows {
        let cells = raw
            .as_array()
            .ok_or_else(|| BoardError::ColumnCountMismatch {
                series: super::error::redact_identifier(&def.name),
                expected: def.columns.len(),
                actual: 0,
            })?;
        if cells.len() != def.columns.len() {
            return Err(BoardError::ColumnCountMismatch {
                series: super::error::redact_identifier(&def.name),
                expected: def.columns.len(),
                actual: cells.len(),
            });
        }

        let mut values = Vec::with_capacity(cells.len());
        for (cell, column) in cells.iter().zip(def.columns.iter()) {
            let value = Value::from_json(cell, column, &def.name)?;
            if let Value::String(s) = &value {
                if s.len() > caps.max_cell_bytes {
                    return Err(BoardError::CapExceeded {
                        what: "bytes in a string cell",
                        limit: caps.max_cell_bytes,
                        actual: s.len(),
                    });
                }
            }
            values.push(value);
        }
        out.push(values);
    }

    Ok(out)
}

/// Detect a gap between a writer's last stored sequence and the next
/// one it claims.
///
/// Returns `None` for the first push from a writer and for a contiguous
/// continuation. A gap is informational, not an error: the rows that
/// arrived are still real, and refusing them would lose data to protect
/// a counter.
pub fn detect_gap(last_seq: Option<i64>, next_seq: i64) -> Option<(i64, i64)> {
    let last = last_seq?;
    if next_seq > last + 1 {
        Some((last + 1, next_seq - 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::series::{Column, ColumnType, WriterKind};

    fn def() -> SeriesDef {
        SeriesDef::new(
            "runs",
            vec![
                Column::new("at", ColumnType::Timestamp),
                Column::new("cost", ColumnType::Number),
                Column::new("note", ColumnType::String),
            ],
        )
    }

    fn req(rows: Vec<serde_json::Value>) -> PushRequest {
        PushRequest {
            board_id: "b".into(),
            series: "runs".into(),
            rows,
            mode: PushMode::Append,
            writer: WriterId::new(WriterKind::AgentRun, "nightly"),
            idem_key: None,
            writer_seq: None,
        }
    }

    #[test]
    fn well_formed_rows_validate() {
        let r = req(vec![serde_json::json!([
            "2026-07-31T00:00:00+00:00",
            1.5,
            "ok"
        ])]);
        let rows = validate_push(&r, &def(), &IngestCaps::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], Value::Number(1.5));
    }

    #[test]
    fn short_row_is_a_column_count_mismatch() {
        let r = req(vec![serde_json::json!(["2026-07-31T00:00:00+00:00", 1.5])]);
        assert!(matches!(
            validate_push(&r, &def(), &IngestCaps::default()),
            Err(BoardError::ColumnCountMismatch {
                expected: 3,
                actual: 2,
                ..
            })
        ));
    }

    #[test]
    fn long_row_is_a_column_count_mismatch() {
        let r = req(vec![serde_json::json!([
            "2026-07-31T00:00:00+00:00",
            1.5,
            "ok",
            "extra"
        ])]);
        assert!(matches!(
            validate_push(&r, &def(), &IngestCaps::default()),
            Err(BoardError::ColumnCountMismatch { actual: 4, .. })
        ));
    }

    #[test]
    fn a_non_array_row_is_rejected() {
        let r = req(vec![serde_json::json!({"at": "x"})]);
        assert!(validate_push(&r, &def(), &IngestCaps::default()).is_err());
    }

    #[test]
    fn rows_per_push_cap_is_enforced() {
        let caps = IngestCaps {
            max_rows_per_push: 2,
            ..IngestCaps::default()
        };
        let rows = (0..3)
            .map(|_| serde_json::json!(["2026-07-31T00:00:00+00:00", 1.0, "x"]))
            .collect();
        assert!(matches!(
            validate_push(&req(rows), &def(), &caps),
            Err(BoardError::CapExceeded {
                what: "rows per push",
                ..
            })
        ));
    }

    #[test]
    fn oversized_string_cell_is_rejected() {
        // Tool output is arbitrary stdout; an unbounded cell turns a
        // board into a log sink.
        let caps = IngestCaps {
            max_cell_bytes: 8,
            ..IngestCaps::default()
        };
        let r = req(vec![serde_json::json!([
            "2026-07-31T00:00:00+00:00",
            1.0,
            "far too long to fit"
        ])]);
        assert!(matches!(
            validate_push(&r, &def(), &caps),
            Err(BoardError::CapExceeded {
                what: "bytes in a string cell",
                ..
            })
        ));
    }

    #[test]
    fn error_text_never_quotes_the_offending_cell() {
        // A board can hold financial records; an error string reaches
        // logs. Same reasoning as corpus::error_signature redaction.
        let r = req(vec![serde_json::json!([
            "2026-07-31T00:00:00+00:00",
            "ACCOUNT-4111111111111111",
            "x"
        ])]);
        let err = validate_push(&r, &def(), &IngestCaps::default()).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("4111111111111111"), "leaked cell: {text}");
        assert!(text.contains("cost"), "should name the column: {text}");
    }

    #[test]
    fn nulls_pass_validation_in_every_column() {
        let r = req(vec![serde_json::json!([null, null, null])]);
        let rows = validate_push(&r, &def(), &IngestCaps::default()).unwrap();
        assert_eq!(rows[0], vec![Value::Null, Value::Null, Value::Null]);
    }

    #[test]
    fn first_push_from_a_writer_reports_no_gap() {
        assert_eq!(detect_gap(None, 1), None);
        assert_eq!(detect_gap(None, 99), None);
    }

    #[test]
    fn contiguous_sequence_reports_no_gap() {
        assert_eq!(detect_gap(Some(4), 5), None);
    }

    #[test]
    fn jumped_sequence_reports_the_missing_range() {
        assert_eq!(detect_gap(Some(4), 8), Some((5, 7)));
        assert_eq!(detect_gap(Some(1), 3), Some((2, 2)));
    }

    #[test]
    fn push_mode_round_trips() {
        for m in [PushMode::Append, PushMode::Replace] {
            assert_eq!(PushMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(PushMode::parse("upsert"), None);
    }

    // ---- shared golden vectors -------------------------------------
    //
    // `testdata/board-series-vectors.json` pins the ingest contract:
    // which rows a series accepts and which it refuses. Following the
    // `rate-resolution-vectors.json` precedent — add a vector whenever
    // a validation rule changes.

    #[derive(serde::Deserialize)]
    struct VectorFile {
        columns: Vec<Column>,
        vectors: Vec<Vector>,
    }

    #[derive(serde::Deserialize)]
    struct Vector {
        name: String,
        row: serde_json::Value,
        expect: String,
    }

    fn vectors_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("board-series-vectors.json")
    }

    #[test]
    fn shared_vectors_match_this_implementation() {
        let raw =
            std::fs::read_to_string(vectors_path()).expect("board-series-vectors.json must exist");
        let file: VectorFile = serde_json::from_str(&raw).expect("vectors must parse");
        assert!(!file.vectors.is_empty(), "fixture must not be empty");

        let def = SeriesDef::new("vectors", file.columns.clone());
        def.validate(IngestCaps::default().max_columns_per_series)
            .expect("fixture series definition must be valid");

        for v in &file.vectors {
            let got = validate_push(&req(vec![v.row.clone()]), &def, &IngestCaps::default());
            match v.expect.as_str() {
                "ok" => {
                    assert!(got.is_ok(), "{}: expected ok, got {:?}", v.name, got.err());
                }
                "type_mismatch" => assert!(
                    matches!(got, Err(BoardError::ColumnTypeMismatch { .. })),
                    "{}: expected a type mismatch, got {got:?}",
                    v.name
                ),
                "column_count" => assert!(
                    matches!(got, Err(BoardError::ColumnCountMismatch { .. })),
                    "{}: expected a column-count mismatch, got {got:?}",
                    v.name
                ),
                "bad_timestamp" => assert!(
                    matches!(got, Err(BoardError::InvalidTimestamp)),
                    "{}: expected an invalid timestamp, got {got:?}",
                    v.name
                ),
                "cap" => assert!(
                    matches!(got, Err(BoardError::CapExceeded { .. })),
                    "{}: expected a cap breach, got {got:?}",
                    v.name
                ),
                other => panic!("{}: unknown expect `{other}`", v.name),
            }
        }
    }

    #[test]
    fn vectors_cover_a_null_in_every_column_type() {
        // The null-is-a-gap rule is the one most likely to be lost in a
        // refactor, so the fixture is required to keep exercising it.
        let raw = std::fs::read_to_string(vectors_path()).unwrap();
        let file: VectorFile = serde_json::from_str(&raw).unwrap();
        let all_null = file.vectors.iter().any(|v| {
            v.expect == "ok"
                && v.row
                    .as_array()
                    .is_some_and(|cells| !cells.is_empty() && cells.iter().all(|c| c.is_null()))
        });
        assert!(all_null, "fixture lost its all-null accepted vector");
    }
}
