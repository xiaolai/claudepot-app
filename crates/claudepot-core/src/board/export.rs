//! Export and import — the re-importable JSON envelope, and
//! CSV-per-series.
//!
//! Plan §12 forbids automatic deletion, which makes export a v1
//! requirement rather than a later convenience: user data with no way
//! out is a trap.
//!
//! # Streaming, not slurping
//!
//! A board has no size ceiling, so [`export_json`] and
//! [`export_csv_dir`] write incrementally through a [`std::io::Write`]
//! sink, reading rows in bounded pages. Materializing a whole board to
//! serialize it is a defect here, not an optimization target.
//!
//! # Provenance survives a round trip, and is still not trusted
//!
//! The envelope carries each row's reported writer. That is a record of
//! what was *claimed*, and preserving it is more honest than rewriting
//! every imported row to say "Import". It grants an attacker nothing:
//! provenance was never authenticated in the first place (see
//! [`super::series::WriterId`]), so a hand-crafted envelope can claim
//! no more than a direct write to the database could.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::BoardError;
use super::series::{check_name, Column};
use super::spec::BoardSpec;
use super::store::BoardStore;

/// Envelope format version. Bumped when the envelope's shape changes,
/// independently of the database's `user_version`.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

/// Rows read per page while streaming. Bounds peak memory without
/// making the query chatty.
const PAGE: usize = 1_000;

/// Stated in every envelope so a reader of the file — not just a reader
/// of this source — knows the provenance fields are claims.
const PROVENANCE_NOTE: &str =
    "Provenance fields record what each writer CLAIMED. Claudepot does not \
     authenticate writers; treat `reported_writer` as unverified.";

/// The envelope header — everything except the rows, which stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardExport {
    pub export_schema_version: u32,
    pub source_board_id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub spec: BoardSpec,
    pub spec_revision: i64,
    pub provenance_note: String,
}

/// A series inside an envelope, rows included. Used on the import side,
/// where the file is already bounded by what was exported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesExport {
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<ExportedRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedRow {
    pub values: Vec<serde_json::Value>,
    pub writer_seq: i64,
    pub reported_writer_kind: String,
    pub reported_writer_label: String,
    pub pushed_at: String,
}

/// The full parsed envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardEnvelope {
    #[serde(flatten)]
    pub header: BoardExport,
    pub series: Vec<SeriesExport>,
}

/// Stream one board to `out` as a JSON envelope.
pub fn export_json<W: Write>(
    store: &BoardStore,
    board_id: &str,
    out: &mut W,
) -> Result<(), BoardError> {
    let board = store.get_board(board_id)?;
    let defs = store.series_defs(board_id)?;

    let header = BoardExport {
        export_schema_version: EXPORT_SCHEMA_VERSION,
        source_board_id: board.board_id.clone(),
        name: board.name.clone(),
        created_at: board.created_at.to_rfc3339(),
        updated_at: board.updated_at.to_rfc3339(),
        spec: board.spec.clone(),
        spec_revision: board.spec_revision,
        provenance_note: PROVENANCE_NOTE.to_string(),
    };

    // Hand-assembled so rows never all exist at once. serde_json's
    // `to_writer` on a fully-built value would defeat the point.
    let header_json = serde_json::to_string(&header)?;
    let trimmed = header_json
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(&header_json);
    write!(out, "{{{trimmed},\"series\":[")?;

    for (i, def) in defs.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(
            out,
            "{{\"name\":{},\"columns\":{},\"rows\":[",
            serde_json::to_string(&def.name)?,
            serde_json::to_string(&def.columns)?
        )?;

        // Upper bound captured before the first page: rows appended
        // mid-export are excluded wholesale rather than partially
        // included. See BoardStore::read_rows_after.
        let upper = store.series_max_row_id(board_id, &def.name)?;
        let mut cursor = 0i64;
        let mut first = true;
        loop {
            let rows = store.read_rows_after(board_id, &def.name, cursor, upper, PAGE)?;
            if rows.is_empty() {
                break;
            }
            for (row_id, row) in &rows {
                cursor = *row_id;
                if !first {
                    out.write_all(b",")?;
                }
                first = false;
                let exported = ExportedRow {
                    values: row.values.iter().map(|v| v.to_json()).collect(),
                    writer_seq: row.writer_seq,
                    reported_writer_kind: row.provenance.writer.kind.as_str().to_string(),
                    reported_writer_label: row.provenance.writer.label.clone(),
                    pushed_at: row.provenance.pushed_at.to_rfc3339(),
                };
                out.write_all(serde_json::to_string(&exported)?.as_bytes())?;
            }
            if rows.len() < PAGE {
                break;
            }
        }
        out.write_all(b"]}")?;
    }

    out.write_all(b"]}")?;
    out.flush()?;
    Ok(())
}

/// Write one CSV per series into `dir`, creating it if absent.
///
/// Provenance columns are prefixed `_reported_` so a spreadsheet reader
/// cannot mistake them for data the agent computed.
pub fn export_csv_dir(
    store: &BoardStore,
    board_id: &str,
    dir: &Path,
) -> Result<Vec<std::path::PathBuf>, BoardError> {
    let defs = store.series_defs(board_id)?;
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();

    for def in &defs {
        // `check_name` already rejects separators, `..`, and a leading
        // dot, so the series name cannot escape `dir`. Re-checked here
        // because this is the call that turns a name into a path.
        check_name(&def.name)?;
        let path = dir.join(format!("{}.csv", def.name));
        let file = std::fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(file);

        let mut header: Vec<String> = def.columns.iter().map(|c| c.name.clone()).collect();
        header.push("_reported_writer_kind".to_string());
        header.push("_reported_writer_label".to_string());
        header.push("_pushed_at".to_string());
        header.push("_writer_seq".to_string());
        writeln!(w, "{}", csv_line(&header))?;

        let upper = store.series_max_row_id(board_id, &def.name)?;
        let mut cursor = 0i64;
        loop {
            let rows = store.read_rows_after(board_id, &def.name, cursor, upper, PAGE)?;
            if rows.is_empty() {
                break;
            }
            for (row_id, row) in &rows {
                cursor = *row_id;
                // Export keeps fidelity; `to_display` redacts. See Value::to_export.
                let mut cells: Vec<String> = row.values.iter().map(|v| v.to_export()).collect();
                cells.push(row.provenance.writer.kind.as_str().to_string());
                cells.push(row.provenance.writer.label.clone());
                cells.push(row.provenance.pushed_at.to_rfc3339());
                cells.push(row.writer_seq.to_string());
                writeln!(w, "{}", csv_line(&cells))?;
            }
            if rows.len() < PAGE {
                break;
            }
        }
        w.flush()?;
        written.push(path);
    }

    Ok(written)
}

/// RFC 4180 quoting. Hand-rolled rather than pulling a `csv`
/// dependency for one writer — see `rules/rust-conventions.md` on
/// dependency hygiene.
///
/// A leading `=`, `+`, `-`, or `@` is prefixed with a single quote:
/// spreadsheet software interprets those as formulas, and a board can
/// hold arbitrary agent output.
fn csv_line(cells: &[String]) -> String {
    cells
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",")
}

fn csv_cell(raw: &str) -> String {
    // Spreadsheet importers skip leading whitespace and control bytes
    // before deciding a cell is a formula, so testing `raw`'s first
    // character alone let `\t=cmd()` and ` =cmd()` through. Test the
    // trimmed form; quote the original.
    let probe = raw.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    let defused = if probe.starts_with(['=', '+', '-', '@']) {
        format!("'{raw}")
    } else {
        raw.to_string()
    };
    if defused.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", defused.replace('"', "\"\""))
    } else {
        defused
    }
}

/// Import an envelope as a **new** board.
///
/// Always allocates a fresh `board_id` and records the original as
/// `source_board_id`. There is deliberately no `--preserve-id`: reusing
/// an id needs a written collision policy, and "it usually doesn't
/// collide" is not one.
pub fn import_json(store: &BoardStore, json: &str) -> Result<String, BoardError> {
    // NOT `?` on serde's error: see BoardError::InvalidEnvelope. The
    // structural detail a caller needs is "it did not match the
    // envelope shape", and serde's rendering of *why* quotes the
    // offending value.
    let envelope: BoardEnvelope = serde_json::from_str(json)
        .map_err(|_| BoardError::InvalidEnvelope("does not match the expected envelope shape"))?;
    if envelope.header.export_schema_version != EXPORT_SCHEMA_VERSION {
        return Err(BoardError::UnsupportedExportVersion {
            found: envelope.header.export_schema_version,
            expected: EXPORT_SCHEMA_VERSION,
        });
    }

    // One transaction for the board, its series, and every row.
    //
    // An earlier version created the board, imported each series
    // separately, and deleted the board on failure. That was not
    // atomic in two ways: the partial board was visible to other
    // processes between commits, and the compensating delete could
    // remove rows another process had written to it in that window.
    // Both dissolve when the whole import is one transaction.
    store.import_board(
        &envelope.header.name,
        &envelope.header.spec,
        &envelope.header.source_board_id,
        &envelope.series,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::ingest::{PushMode, PushRequest};
    use crate::board::series::SeriesDef;
    use crate::board::series::{ColumnType, Value, WriterId, WriterKind};

    fn seeded() -> (tempfile::TempDir, BoardStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(&dir.path().join("boards.db")).unwrap();
        let defs = vec![SeriesDef::new(
            "runs",
            vec![
                Column::new("at", ColumnType::Timestamp),
                Column::new("cost", ColumnType::Number),
                Column::new("note", ColumnType::String),
            ],
        )];
        let board = store
            .create_board("nightly", &BoardSpec::empty(), &defs)
            .unwrap();
        store
            .push(&PushRequest {
                board_id: board.board_id.clone(),
                series: "runs".into(),
                rows: vec![
                    serde_json::json!(["2026-07-31T00:00:00+00:00", 1.5, "ok"]),
                    serde_json::json!(["2026-07-31T01:00:00+00:00", null, "gap"]),
                ],
                mode: PushMode::Append,
                writer: WriterId::new(WriterKind::AgentRun, "nightly"),
                idem_key: None,
                writer_seq: None,
            })
            .unwrap();
        let id = board.board_id.clone();
        (dir, store, id)
    }

    #[test]
    fn json_export_is_valid_json_with_the_expected_shape() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let parsed: BoardEnvelope = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed.header.export_schema_version, EXPORT_SCHEMA_VERSION);
        assert_eq!(parsed.header.source_board_id, id);
        assert_eq!(parsed.series.len(), 1);
        assert_eq!(parsed.series[0].rows.len(), 2);
    }

    #[test]
    fn envelope_states_that_provenance_is_unverified() {
        // A reader of the *file*, not just of this source, must be able
        // to tell.
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("unverified"), "note missing: {text}");
        assert!(text.contains("reported_writer_kind"));
    }

    #[test]
    fn export_import_round_trips_rows_and_types() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let json = String::from_utf8(buf).unwrap();

        let new_id = import_json(&store, &json).unwrap();
        assert_ne!(new_id, id);

        let rows = store.read_rows(&new_id, "runs", 100).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[1], Value::Number(1.5));
        // A gap must survive as a gap, not become zero.
        assert_eq!(rows[1].values[1], Value::Null);
    }

    #[test]
    fn import_allocates_a_new_id_and_keeps_the_source() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let new_id = import_json(&store, &String::from_utf8(buf).unwrap()).unwrap();
        assert_ne!(new_id, id);
        assert_eq!(
            store.source_board_id(&new_id).unwrap().as_deref(),
            Some(id.as_str())
        );
        // The original is untouched.
        assert!(store.get_board(&id).is_ok());
    }

    #[test]
    fn import_preserves_reported_provenance_without_upgrading_it() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let new_id = import_json(&store, &String::from_utf8(buf).unwrap()).unwrap();
        let rows = store.read_rows(&new_id, "runs", 10).unwrap();
        assert_eq!(rows[0].provenance.writer.label, "nightly");
        assert!(!rows[0].provenance.verified);
    }

    #[test]
    fn import_rejects_an_unknown_envelope_version() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let bumped = String::from_utf8(buf).unwrap().replace(
            "\"export_schema_version\":1",
            "\"export_schema_version\":99",
        );
        assert!(matches!(
            import_json(&store, &bumped),
            Err(BoardError::UnsupportedExportVersion { found: 99, .. })
        ));
    }

    #[test]
    fn csv_export_writes_one_file_per_series_with_a_header() {
        let (_d, store, id) = seeded();
        let out = tempfile::tempdir().unwrap();
        let written = export_csv_dir(&store, &id, out.path()).unwrap();
        assert_eq!(written.len(), 1);
        let text = std::fs::read_to_string(&written[0]).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            "at,cost,note,_reported_writer_kind,_reported_writer_label,_pushed_at,_writer_seq"
        );
        assert_eq!(lines.count(), 2);
    }

    #[test]
    fn csv_quotes_separators_and_embedded_quotes() {
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn csv_defuses_formula_injection() {
        // A board can hold arbitrary agent output, and a cell starting
        // with `=` is executed by Excel and Sheets on open.
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("+SUM(A1)"), "'+SUM(A1)");
        assert_eq!(csv_cell("-2"), "'-2");
        assert_eq!(csv_cell("@import"), "'@import");
    }

    #[test]
    fn csv_defusing_sees_past_leading_whitespace_and_control_bytes() {
        // Spreadsheet importers skip these before deciding a cell is a
        // formula, so testing only the first raw byte let them through.
        for raw in ["\t=cmd()", " =cmd()", "\r=cmd()", "\u{0b}=cmd()", "  \t@x"] {
            let out = csv_cell(raw);
            assert!(
                out.starts_with('\'') || out.starts_with("\"'"),
                "not defused: {raw:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn a_failed_series_import_leaves_no_partial_board_behind() {
        // Board creation and row import are separate transactions, so
        // the failure path has to undo the board explicitly.
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let before = store.list_boards().unwrap().len();

        // Corrupt a row's timestamp so import fails partway.
        let broken = String::from_utf8(buf)
            .unwrap()
            .replace("\"pushed_at\":\"2026", "\"pushed_at\":\"nonsense-2026");
        assert!(matches!(
            import_json(&store, &broken),
            Err(BoardError::CorruptRow { .. })
        ));

        assert_eq!(
            store.list_boards().unwrap().len(),
            before,
            "a half-imported board survived"
        );
    }

    #[test]
    fn import_rejects_an_out_of_range_writer_sequence() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let tampered = String::from_utf8(buf)
            .unwrap()
            .replace("\"writer_seq\":1", "\"writer_seq\":9223372036854775807");
        assert!(
            matches!(
                import_json(&store, &tampered),
                Err(BoardError::SequenceOutOfRange { .. })
            ),
            "i64::MAX sequence accepted, or rejected for the wrong reason"
        );
    }

    #[test]
    fn import_is_atomic_across_series() {
        // A failure in the *second* series must leave nothing at all —
        // not a board with the first series' rows in it.
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(&dir.path().join("boards.db")).unwrap();
        let defs = vec![
            SeriesDef::new("a", vec![Column::new("n", ColumnType::Integer)]),
            SeriesDef::new("b", vec![Column::new("n", ColumnType::Integer)]),
        ];
        let board = store
            .create_board("two", &BoardSpec::empty(), &defs)
            .unwrap();
        for name in ["a", "b"] {
            store
                .push(&PushRequest {
                    board_id: board.board_id.clone(),
                    series: name.into(),
                    rows: vec![serde_json::json!([1])],
                    mode: PushMode::Append,
                    writer: WriterId::new(WriterKind::Cli, "seed"),
                    idem_key: None,
                    writer_seq: None,
                })
                .unwrap();
        }

        let mut buf = Vec::new();
        export_json(&store, &board.board_id, &mut buf).unwrap();
        let mut env: BoardEnvelope = serde_json::from_slice(&buf).unwrap();
        // Break only the second series.
        env.series[1].rows[0].values = vec![serde_json::json!("not-an-integer")];
        let broken = serde_json::to_string(&env).unwrap();

        let before = store.list_boards().unwrap().len();
        assert!(matches!(
            import_json(&store, &broken),
            Err(BoardError::ColumnTypeMismatch { .. })
        ));
        assert_eq!(
            store.list_boards().unwrap().len(),
            before,
            "a partially imported board was committed"
        );
    }

    #[test]
    fn import_enforces_the_same_cell_size_cap_as_push() {
        // The two paths drifted here once already: import skipped a
        // limit the write path applied.
        let dir = tempfile::tempdir().unwrap();
        let caps = claudepot_core_caps();
        let store = BoardStore::open_with_caps(&dir.path().join("boards.db"), caps).unwrap();
        let defs = vec![SeriesDef::new(
            "s",
            vec![Column::new("t", ColumnType::String)],
        )];
        let board = store.create_board("c", &BoardSpec::empty(), &defs).unwrap();
        store
            .push(&PushRequest {
                board_id: board.board_id.clone(),
                series: "s".into(),
                rows: vec![serde_json::json!(["ok"])],
                mode: PushMode::Append,
                writer: WriterId::new(WriterKind::Cli, "seed"),
                idem_key: None,
                writer_seq: None,
            })
            .unwrap();

        let mut buf = Vec::new();
        export_json(&store, &board.board_id, &mut buf).unwrap();
        let oversized = String::from_utf8(buf)
            .unwrap()
            .replace("\"ok\"", &format!("\"{}\"", "x".repeat(64)));
        assert!(
            matches!(
                import_json(&store, &oversized),
                Err(BoardError::CapExceeded {
                    what: "bytes in a string cell",
                    ..
                })
            ),
            "import accepted a cell push would have rejected, or rejected it for another reason"
        );
    }

    #[test]
    fn envelope_parse_failure_does_not_echo_the_offending_value() {
        // serde renders "unknown variant `X`" with X verbatim, and an
        // envelope is untrusted input — so a token-shaped ColumnType
        // would print straight past redact_identifier.
        let (_d, store, _id) = seeded();
        let poisoned = r#"{"export_schema_version":1,"source_board_id":"x","name":"n",
            "created_at":"2026-07-31T00:00:00+00:00","updated_at":"2026-07-31T00:00:00+00:00",
            "spec":{"widgets":[]},"spec_revision":1,"provenance_note":"",
            "series":[{"name":"s","columns":[{"name":"c","type":"sk-ant-oat01-LEAKME"}],"rows":[]}]}"#;
        let err = import_json(&store, poisoned).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("LEAKME"), "envelope value leaked: {text}");
        assert!(matches!(err, BoardError::InvalidEnvelope(_)));
    }

    #[test]
    fn appending_an_envelope_counts_rows_already_in_the_series() {
        // import_board's cap check is enough for a fresh series;
        // appending into an existing one has to count what is there, or
        // repeated imports walk past the cap one envelope at a time.
        let dir = tempfile::tempdir().unwrap();
        let caps = crate::board::IngestCaps {
            max_rows_per_series: 3,
            ..crate::board::IngestCaps::default()
        };
        let store = BoardStore::open_with_caps(&dir.path().join("boards.db"), caps).unwrap();
        let defs = vec![SeriesDef::new(
            "s",
            vec![Column::new("n", ColumnType::Integer)],
        )];
        let board = store
            .create_board("cap", &BoardSpec::empty(), &defs)
            .unwrap();
        store
            .push(&PushRequest {
                board_id: board.board_id.clone(),
                series: "s".into(),
                rows: vec![serde_json::json!([1]), serde_json::json!([2])],
                mode: PushMode::Append,
                writer: WriterId::new(WriterKind::Cli, "seed"),
                idem_key: None,
                writer_seq: None,
            })
            .unwrap();

        let mut buf = Vec::new();
        export_json(&store, &board.board_id, &mut buf).unwrap();
        let envelope: BoardEnvelope = serde_json::from_slice(&buf).unwrap();

        // 2 existing + 2 imported = 4, over the cap of 3.
        assert!(matches!(
            store.import_rows(&board.board_id, &envelope.series[0]),
            Err(BoardError::CapExceeded {
                what: "rows per series",
                ..
            })
        ));
        assert_eq!(store.row_count(&board.board_id, "s").unwrap(), 2);
    }

    fn claudepot_core_caps() -> crate::board::IngestCaps {
        crate::board::IngestCaps {
            max_cell_bytes: 8,
            ..crate::board::IngestCaps::default()
        }
    }

    #[test]
    fn import_rejects_an_unparseable_pushed_at() {
        let (_d, store, id) = seeded();
        let mut buf = Vec::new();
        export_json(&store, &id, &mut buf).unwrap();
        let tampered = String::from_utf8(buf)
            .unwrap()
            .replace("\"pushed_at\":\"2026", "\"pushed_at\":\"whenever-2026");
        assert!(matches!(
            import_json(&store, &tampered),
            Err(BoardError::CorruptRow { .. })
        ));
    }

    #[test]
    fn csv_renders_a_null_as_a_gap() {
        let (_d, store, id) = seeded();
        let out = tempfile::tempdir().unwrap();
        let written = export_csv_dir(&store, &id, out.path()).unwrap();
        let text = std::fs::read_to_string(&written[0]).unwrap();
        let second = text.lines().nth(2).unwrap();
        assert!(second.contains("—"), "null lost its gap: {second}");
        assert!(!second.contains(",0,"), "null became zero: {second}");
    }

    #[test]
    fn export_streams_across_page_boundaries() {
        // Exercises the paging loop rather than trusting one short read.
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(&dir.path().join("boards.db")).unwrap();
        let defs = vec![SeriesDef::new(
            "n",
            vec![Column::new("i", ColumnType::Integer)],
        )];
        let board = store
            .create_board("big", &BoardSpec::empty(), &defs)
            .unwrap();
        let rows: Vec<serde_json::Value> = (0..PAGE + 250)
            .map(|i| serde_json::json!([i as i64]))
            .collect();
        store
            .push(&PushRequest {
                board_id: board.board_id.clone(),
                series: "n".into(),
                rows,
                mode: PushMode::Append,
                writer: WriterId::new(WriterKind::Cli, "seed"),
                idem_key: None,
                writer_seq: None,
            })
            .unwrap();

        let mut buf = Vec::new();
        export_json(&store, &board.board_id, &mut buf).unwrap();
        let parsed: BoardEnvelope = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed.series[0].rows.len(), PAGE + 250);
    }
}
