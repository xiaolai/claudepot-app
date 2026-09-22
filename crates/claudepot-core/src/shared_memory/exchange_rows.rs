//! The one writer for a transcript's `exchanges` + `tool_calls` rows.
//!
//! Both backfills (Claude and Codex) used to `DELETE FROM exchanges WHERE
//! file_path = ?` and re-insert every row whenever a transcript's
//! `(size, mtime, inode)` moved. A live transcript moves on every pass, so a
//! 263 MB one was rewritten in full every two minutes. macOS attributed
//! 34.36 GB of writes in 20 hours to the GUI, 80% of it inside the Claude
//! backfill's inserts and commit.
//!
//! [`reconcile`] makes the file's rows equal to a fresh parse while writing
//! only what differs. A row the parse reproduces exactly is not touched — an
//! `UPDATE` whose `WHERE` finds no difference dirties no page — a changed row
//! is updated in place, a row the parse no longer produces is deleted, and a
//! new one is inserted. The end state is the one delete-and-reinsert
//! produced, which is what the tests pin. Two things follow from touching
//! less:
//!
//! * an append costs writes in proportion to what was appended, plus the
//!   still-open exchange, whose text the FTS trigger re-indexes whole;
//! * a `memory_links` row that points at an exchange survives a re-index for
//!   as long as the exchange does. Delete-and-reinsert cascaded every such
//!   link away on every pass, although the same exchange id came straight
//!   back.
//!
//! Row ids are the caller's. Both callers derive them from the file, so a
//! file's ids never collide with another file's; a new id that does collide
//! fails its `INSERT` on the primary key, exactly as before.

use std::collections::HashSet;

use rusqlite::{params, Connection};

/// One `exchanges` row as a parse produced it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExchangeRow {
    pub(crate) id: String,
    pub(crate) turn_index: i64,
    pub(crate) timestamp_ms: Option<i64>,
    pub(crate) user_text: String,
    pub(crate) assistant_text: String,
    pub(crate) line_start: Option<i64>,
    pub(crate) line_end: Option<i64>,
    pub(crate) snippet_text: String,
    pub(crate) tool_calls: Vec<ToolCallRow>,
}

/// One `tool_calls` row, belonging to the [`ExchangeRow`] that holds it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCallRow {
    pub(crate) id: String,
    pub(crate) tool_name: String,
    pub(crate) tool_input_json: Option<String>,
    pub(crate) tool_result_text: Option<String>,
    pub(crate) is_error: bool,
    pub(crate) timestamp_ms: Option<i64>,
}

/// Rows actually written by one [`reconcile`], per table. A pass over a
/// file whose parse reproduces the stored rows reports all zeros.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RowWrites {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
}

impl RowWrites {
    pub fn total(&self) -> usize {
        self.inserted + self.updated + self.deleted
    }
}

/// What [`reconcile`] wrote.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileWrites {
    pub exchanges: RowWrites,
    pub tool_calls: RowWrites,
}

impl ReconcileWrites {
    pub fn total(&self) -> usize {
        self.exchanges.total() + self.tool_calls.total()
    }
}

// Every column is compared with `IS NOT` so NULL-vs-value counts as a
// difference and NULL-vs-NULL does not. `file_path` is not set: an id
// present for this file stays with this file.
const SQL_UPDATE_EXCHANGE: &str = "\
UPDATE exchanges SET
    source_kind = ?2, turn_index = ?3, role_pair = 'user_assistant',
    timestamp_ms = ?4, user_text = ?5, assistant_text = ?6,
    line_start = ?7, line_end = ?8, is_sidechain = 0, parent_id = NULL,
    snippet_text = ?9
WHERE id = ?1 AND (
    source_kind IS NOT ?2 OR turn_index IS NOT ?3
    OR role_pair IS NOT 'user_assistant'
    OR timestamp_ms IS NOT ?4 OR user_text IS NOT ?5
    OR assistant_text IS NOT ?6 OR line_start IS NOT ?7
    OR line_end IS NOT ?8 OR is_sidechain IS NOT 0
    OR parent_id IS NOT NULL OR snippet_text IS NOT ?9
)";

const SQL_INSERT_EXCHANGE: &str = "\
INSERT INTO exchanges (
    id, file_path, source_kind, turn_index, role_pair,
    timestamp_ms, user_text, assistant_text,
    line_start, line_end, is_sidechain, parent_id, snippet_text
) VALUES (?1, ?2, ?3, ?4, 'user_assistant', ?5, ?6, ?7, ?8, ?9, 0, NULL, ?10)";

const SQL_UPDATE_TOOL_CALL: &str = "\
UPDATE tool_calls SET
    exchange_id = ?2, tool_name = ?3, tool_input_json = ?4,
    tool_result_text = ?5, is_error = ?6, timestamp_ms = ?7
WHERE id = ?1 AND (
    exchange_id IS NOT ?2 OR tool_name IS NOT ?3
    OR tool_input_json IS NOT ?4 OR tool_result_text IS NOT ?5
    OR is_error IS NOT ?6 OR timestamp_ms IS NOT ?7
)";

const SQL_INSERT_TOOL_CALL: &str = "\
INSERT INTO tool_calls (
    id, exchange_id, tool_name, tool_input_json,
    tool_result_text, is_error, timestamp_ms
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

/// Make `file_path`'s `exchanges` and `tool_calls` rows equal to `rows`,
/// writing only the difference. Run it inside the caller's transaction:
/// it reads the stored ids first, and the result is only consistent if
/// nothing else writes the file's rows in between.
pub(crate) fn reconcile(
    conn: &Connection,
    file_path: &str,
    source_kind: &str,
    rows: &[ExchangeRow],
) -> rusqlite::Result<ReconcileWrites> {
    let mut writes = ReconcileWrites::default();

    let stored_exchanges: HashSet<String> = {
        let mut stmt = conn.prepare_cached("SELECT id FROM exchanges WHERE file_path = ?1")?;
        let ids = stmt.query_map([file_path], |r| r.get::<_, String>(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };
    let stored_tool_calls: HashSet<String> = {
        let mut stmt = conn.prepare_cached(
            "SELECT tc.id FROM tool_calls tc \
             JOIN exchanges e ON e.id = tc.exchange_id \
             WHERE e.file_path = ?1",
        )?;
        let ids = stmt.query_map([file_path], |r| r.get::<_, String>(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };

    let parsed_exchanges: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let parsed_tool_calls: HashSet<&str> = rows
        .iter()
        .flat_map(|r| r.tool_calls.iter().map(|t| t.id.as_str()))
        .collect();

    // Stale exchanges first. The FK cascade takes their tool calls and
    // links, and the AFTER DELETE trigger takes their FTS rows.
    {
        let mut stmt = conn.prepare_cached("DELETE FROM exchanges WHERE id = ?1")?;
        for id in stored_exchanges
            .iter()
            .filter(|id| !parsed_exchanges.contains(id.as_str()))
        {
            writes.exchanges.deleted += stmt.execute([id])?;
        }
    }
    {
        let mut stmt = conn.prepare_cached("DELETE FROM tool_calls WHERE id = ?1")?;
        for id in stored_tool_calls
            .iter()
            .filter(|id| !parsed_tool_calls.contains(id.as_str()))
        {
            // Zero when the cascade above already took it.
            writes.tool_calls.deleted += stmt.execute([id])?;
        }
    }

    let mut update_exchange = conn.prepare_cached(SQL_UPDATE_EXCHANGE)?;
    let mut insert_exchange = conn.prepare_cached(SQL_INSERT_EXCHANGE)?;
    let mut update_tool_call = conn.prepare_cached(SQL_UPDATE_TOOL_CALL)?;
    let mut insert_tool_call = conn.prepare_cached(SQL_INSERT_TOOL_CALL)?;
    for ex in rows {
        if stored_exchanges.contains(&ex.id) {
            writes.exchanges.updated += update_exchange.execute(params![
                ex.id,
                source_kind,
                ex.turn_index,
                ex.timestamp_ms,
                ex.user_text,
                ex.assistant_text,
                ex.line_start,
                ex.line_end,
                ex.snippet_text,
            ])?;
        } else {
            writes.exchanges.inserted += insert_exchange.execute(params![
                ex.id,
                file_path,
                source_kind,
                ex.turn_index,
                ex.timestamp_ms,
                ex.user_text,
                ex.assistant_text,
                ex.line_start,
                ex.line_end,
                ex.snippet_text,
            ])?;
        }
        for tc in &ex.tool_calls {
            let is_error = tc.is_error as i64;
            if stored_tool_calls.contains(&tc.id) {
                writes.tool_calls.updated += update_tool_call.execute(params![
                    tc.id,
                    ex.id,
                    tc.tool_name,
                    tc.tool_input_json,
                    tc.tool_result_text,
                    is_error,
                    tc.timestamp_ms,
                ])?;
            } else {
                writes.tool_calls.inserted += insert_tool_call.execute(params![
                    tc.id,
                    ex.id,
                    tc.tool_name,
                    tc.tool_input_json,
                    tc.tool_result_text,
                    is_error,
                    tc.timestamp_ms,
                ])?;
            }
        }
    }

    Ok(writes)
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;

    /// Everything `reconcile` is responsible for, in a form two databases
    /// can be compared by: every exchange column, every tool-call column,
    /// and which exchange each FTS term finds.
    pub(crate) type Snapshot = (Vec<String>, Vec<String>, Vec<(String, Vec<String>)>);

    pub(crate) fn snapshot(conn: &Connection, terms: &[&str]) -> Snapshot {
        let rows = |sql: &str| -> Vec<String> {
            let mut stmt = conn.prepare(sql).unwrap();
            let n = stmt.column_count();
            stmt.query_map([], |r| {
                Ok((0..n)
                    .map(|i| match r.get_ref(i).unwrap() {
                        rusqlite::types::ValueRef::Null => "NULL".to_string(),
                        rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                        rusqlite::types::ValueRef::Real(v) => v.to_string(),
                        // Verbatim, not `{:?}`: Debug doubles every `\`,
                        // so a Windows path would no longer match the
                        // raw path callers normalize away.
                        rusqlite::types::ValueRef::Text(t) => {
                            format!("text:{}", String::from_utf8_lossy(t))
                        }
                        rusqlite::types::ValueRef::Blob(b) => format!("blob{b:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
        };
        let exchanges = rows(
            "SELECT id, file_path, source_kind, turn_index, role_pair, timestamp_ms, \
             user_text, assistant_text, line_start, line_end, is_sidechain, parent_id, \
             snippet_text FROM exchanges ORDER BY id",
        );
        let tool_calls = rows(
            "SELECT id, exchange_id, tool_name, tool_input_json, tool_result_text, \
             is_error, timestamp_ms FROM tool_calls ORDER BY id",
        );
        // External-content FTS5 can silently diverge from its table; this
        // is the check that it has not.
        conn.execute(
            "INSERT INTO exchange_fts(exchange_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .expect("exchange_fts must agree with exchanges");
        let hits = terms
            .iter()
            .map(|t| {
                let mut stmt = conn
                    .prepare(
                        "SELECT e.id FROM exchange_fts f JOIN exchanges e ON e.rowid = f.rowid \
                         WHERE exchange_fts MATCH ?1 ORDER BY e.id",
                    )
                    .unwrap();
                let ids = stmt
                    .query_map([t], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (t.to_string(), ids)
            })
            .collect();
        (exchanges, tool_calls, hits)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{snapshot, Snapshot};
    use super::*;
    use crate::session_index::SessionIndex;
    use tempfile::TempDir;

    /// An index holding one `sessions` row for the rows to hang off
    /// (`exchanges.file_path` is a foreign key onto it).
    fn fixture() -> (TempDir, SessionIndex, String) {
        let tmp = TempDir::new().unwrap();
        let idx = SessionIndex::open(&tmp.path().join("sessions.db")).unwrap();
        let cfg = tmp.path().join("claude");
        let dir = cfg.join("projects").join("-p");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\
             \"timestamp\":\"2026-05-15T11:30:00.000Z\",\"sessionId\":\"s\",\"cwd\":\"/p\"}\n",
        )
        .unwrap();
        idx.refresh(&cfg).unwrap();
        (tmp, idx, file.to_string_lossy().into_owned())
    }

    fn tool(exchange: &str, ordinal: usize, result: Option<&str>) -> ToolCallRow {
        ToolCallRow {
            id: format!("{exchange}\u{1f}{ordinal}\u{1f}tu_{ordinal}"),
            tool_name: "Bash".into(),
            tool_input_json: Some(format!("{{\"command\":\"step {ordinal}\"}}")),
            tool_result_text: result.map(str::to_string),
            is_error: false,
            timestamp_ms: Some(1_000 + ordinal as i64),
        }
    }

    fn exchange(turn: i64, user: &str, assistant: &str, tools: Vec<ToolCallRow>) -> ExchangeRow {
        ExchangeRow {
            id: format!("claude_code:-p/s:{turn}"),
            turn_index: turn,
            timestamp_ms: Some(turn * 10),
            user_text: user.into(),
            assistant_text: assistant.into(),
            line_start: None,
            line_end: None,
            snippet_text: format!("{user} → {assistant}"),
            tool_calls: tools,
        }
    }

    fn id(turn: i64) -> String {
        format!("claude_code:-p/s:{turn}")
    }

    /// Successive parses of one transcript: it grows (a result lands, the
    /// open exchange's prose grows, a tool call and an exchange appear),
    /// then is rewritten shorter with a redacted word — the `slim` /
    /// `redact` shape.
    fn versions() -> Vec<Vec<ExchangeRow>> {
        let e0 = id(0);
        let e1 = id(1);
        vec![
            vec![exchange(
                0,
                "refactor auth",
                "reading",
                vec![tool(&e0, 0, None)],
            )],
            vec![
                exchange(
                    0,
                    "refactor auth",
                    "reading\nzebrafish found",
                    vec![tool(&e0, 0, Some("file body")), tool(&e0, 1, None)],
                ),
                exchange(1, "now the tests", "running", vec![tool(&e1, 0, Some(""))]),
            ],
            vec![exchange(
                0,
                "refactor auth",
                "reading\n[REDACTED] found",
                vec![tool(&e0, 0, Some("file body"))],
            )],
        ]
    }

    const TERMS: &[&str] = &["reading", "zebrafish", "redacted", "tests", "auth"];

    #[test]
    fn reconcile_converges_to_what_a_fresh_index_holds() {
        let (_a_dir, a, file) = fixture();
        for (n, rows) in versions().iter().enumerate() {
            {
                let db = a.db();
                reconcile(&db, &file, "claude_code", rows).unwrap();
            }
            // A fresh index of this version alone: what delete-and-reinsert
            // produced, and what the incremental history must equal.
            let (_b_dir, b, b_file) = fixture();
            {
                let db = b.db();
                reconcile(&db, &b_file, "claude_code", rows).unwrap();
            }
            let normalize = |s: Snapshot, path: &str| -> Snapshot {
                let strip =
                    |v: Vec<String>| v.into_iter().map(|r| r.replace(path, "<file>")).collect();
                (strip(s.0), strip(s.1), s.2)
            };
            assert_eq!(
                normalize(snapshot(&a.db(), TERMS), &file),
                normalize(snapshot(&b.db(), TERMS), &b_file),
                "after version {n}, the reconciled rows differ from a fresh index"
            );
        }
        // The redaction reached the text index, not just the table.
        let hits = snapshot(&a.db(), TERMS).2;
        assert_eq!(
            hits.iter().find(|(t, _)| t == "zebrafish").unwrap().1,
            Vec::<String>::new()
        );
        assert_eq!(
            hits.iter().find(|(t, _)| t == "tests").unwrap().1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn reconciling_the_same_parse_again_writes_nothing() {
        let (_dir, idx, file) = fixture();
        let rows = &versions()[1];
        let db = idx.db();
        let first = reconcile(&db, &file, "claude_code", rows).unwrap();
        assert!(first.total() > 0);
        let changes_before = db.total_changes();
        let again = reconcile(&db, &file, "claude_code", rows).unwrap();
        assert_eq!(again, ReconcileWrites::default());
        assert_eq!(db.total_changes(), changes_before, "no row may be touched");
    }

    #[test]
    fn an_append_writes_the_appended_rows_and_nothing_else() {
        let (_dir, idx, file) = fixture();
        let e0 = id(0);
        let e1 = id(1);
        let before = vec![exchange(0, "a", "b", vec![tool(&e0, 0, Some("r0"))])];
        let after = vec![
            exchange(
                0,
                "a",
                "b",
                vec![tool(&e0, 0, Some("r0")), tool(&e0, 1, Some("r1"))],
            ),
            exchange(1, "c", "d", vec![tool(&e1, 0, None)]),
        ];
        let db = idx.db();
        reconcile(&db, &file, "claude_code", &before).unwrap();
        let writes = reconcile(&db, &file, "claude_code", &after).unwrap();
        assert_eq!(
            writes,
            ReconcileWrites {
                exchanges: RowWrites {
                    inserted: 1,
                    updated: 0,
                    deleted: 0
                },
                tool_calls: RowWrites {
                    inserted: 2,
                    updated: 0,
                    deleted: 0
                },
            }
        );
    }

    #[test]
    fn a_link_survives_for_as_long_as_its_exchange_does() {
        // Delete-and-reinsert cascaded every `memory_links` row pointing at
        // an exchange away on every pass, although the same id came
        // straight back.
        let (_dir, idx, file) = fixture();
        let v = versions();
        let db = idx.db();
        reconcile(&db, &file, "claude_code", &v[1]).unwrap();
        db.execute(
            "INSERT INTO memories (id, scope, kind, content, created_by_kind, created_by, \
             created_at_ms, updated_at_ms) VALUES ('m', 'global', 'fact', 'x', 'user', 't', 0, 0)",
            [],
        )
        .unwrap();
        for (link, target) in [("l0", id(0)), ("l1", id(1))] {
            db.execute(
                "INSERT INTO memory_links (id, memory_id, exchange_id, relation) \
                 VALUES (?1, 'm', ?2, 'evidence')",
                params![link, target],
            )
            .unwrap();
        }
        // Exchange 0 changes (and survives); exchange 1 is gone.
        reconcile(&db, &file, "claude_code", &v[2]).unwrap();
        let links: Vec<String> = db
            .prepare("SELECT id FROM memory_links ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(links, vec!["l0".to_string()]);
    }

    #[test]
    fn an_id_owned_by_another_file_still_fails_on_the_primary_key() {
        // Ids are the caller's; a colliding new id must not be silently
        // re-pointed at this file.
        let (dir, idx, file) = fixture();
        let cfg = dir.path().join("claude");
        let other = cfg.join("projects").join("-p").join("other.jsonl");
        std::fs::copy(&file, &other).unwrap();
        idx.refresh(&cfg).unwrap();
        let other = other.to_string_lossy().into_owned();
        let db = idx.db();
        let rows = vec![exchange(0, "a", "b", vec![])];
        reconcile(&db, &file, "claude_code", &rows).unwrap();
        let err = reconcile(&db, &other, "claude_code", &rows).unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "got {err}");
    }
}
