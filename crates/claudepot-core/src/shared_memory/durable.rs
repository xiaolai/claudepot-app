//! CRUD over `memories`, `decisions`, `evidence_records`,
//! `memory_links` (WI-006).
//!
//! These four tables hold user/agent-authored durable rows. They
//! are NOT cascade-cleared by "Rebuild Shared Memory" — only
//! "Forget Shared Memory" wipes them.
//!
//! Authorship encoding follows D9 in the plan: every durable row
//! carries `created_by_kind` ∈ {user, agent, import, system} and
//! a free-form `created_by` actor id (e.g. `codex@2026-05-15`,
//! `claude-code`, `user:xiaolai`). The transcript-derived
//! `source_kind` column lives on `exchanges`, never on these
//! durable tables.
//!
//! ## Rate-limiting and duplicate detection (L13)
//!
//! The schema deliberately does NOT enforce a UNIQUE constraint on
//! `(scope, project_path, content)` for memories. Legitimate use
//! cases include the same fact captured by different agents on
//! different days, or by the user explicitly reaffirming a
//! preference — both should land as distinct rows for audit
//! purposes. The trade-off: a misbehaving agent can write
//! unlimited duplicates.
//!
//! Mitigation lives one layer up:
//!   * MCP boundary: future per-session rate limit on `remember`
//!     and `log_decision` calls (not in this slice — track via the
//!     dedup-counter in `claudepot mcp memory-server` once we see
//!     real misbehavior).
//!   * UI: "duplicates of X exist" surfacing in the Shared Memory
//!     section so the user can prune.
//!
//! Application layer remains responsible for noticing
//! pathological patterns; the data layer is permissive.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session_index::SessionIndex;

// ─── enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Project,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Fact,
    Preference,
    Pattern,
    Constraint,
    Summary,
}

impl MemoryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Pattern => "pattern",
            Self::Constraint => "constraint",
            Self::Summary => "summary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedByKind {
    User,
    Agent,
    Import,
    System,
}

impl CreatedByKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Import => "import",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Active,
    Superseded,
    Archived,
}

impl DecisionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkRelation {
    Evidence,
    Origin,
    Related,
    Supersedes,
}

impl LinkRelation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Origin => "origin",
            Self::Related => "related",
            Self::Supersedes => "supersedes",
        }
    }
}

// ─── records ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: Scope,
    pub project_path: Option<String>,
    pub kind: MemoryKind,
    pub content: String,
    pub created_by_kind: CreatedByKind,
    pub created_by: String,
    pub confidence: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub archived_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRecord {
    pub id: String,
    pub project_path: Option<String>,
    pub topic: Option<String>,
    pub decision: String,
    pub rationale: Option<String>,
    pub status: DecisionStatus,
    pub created_by_kind: CreatedByKind,
    pub created_by: String,
    pub created_at_ms: i64,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub id: String,
    pub project_path: Option<String>,
    pub topic: Option<String>,
    pub summary: String,
    pub verification: String,
    pub files_changed_json: String,
    pub confidence: i64,
    pub created_by_kind: CreatedByKind,
    pub created_by: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLinkRecord {
    pub id: String,
    pub memory_id: Option<String>,
    pub decision_id: Option<String>,
    pub evidence_id: Option<String>,
    pub exchange_id: Option<String>,
    pub file_path: Option<String>,
    pub relation: LinkRelation,
}

// ─── new-record builders (caller-friendly) ────────────────────

/// Arguments for `create_memory`. Most fields are optional; the
/// store fills in `id`, `created_at_ms`, `updated_at_ms`.
#[derive(Debug, Clone)]
pub struct NewMemory<'a> {
    pub scope: Scope,
    pub project_path: Option<&'a str>,
    pub kind: MemoryKind,
    pub content: &'a str,
    pub created_by_kind: CreatedByKind,
    pub created_by: &'a str,
    pub confidence: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewDecision<'a> {
    pub project_path: Option<&'a str>,
    pub topic: Option<&'a str>,
    pub decision: &'a str,
    pub rationale: Option<&'a str>,
    pub created_by_kind: CreatedByKind,
    pub created_by: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewEvidence<'a> {
    pub project_path: Option<&'a str>,
    pub topic: Option<&'a str>,
    pub summary: &'a str,
    pub verification: &'a str,
    pub files_changed_json: &'a str,
    pub confidence: i64,
    pub created_by_kind: CreatedByKind,
    pub created_by: &'a str,
}

// ─── error ────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DurableError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("invalid scope/project combination: scope={scope:?} project_path={project_path:?}")]
    InvalidScope {
        scope: Scope,
        project_path: Option<String>,
    },

    #[error("decision not found: {0}")]
    DecisionNotFound(String),
}

// ─── transaction helper ───────────────────────────────────────

/// Run `f` inside a SQLite transaction. Commit on Ok, roll back
/// (via drop) on Err. Every writer in this module goes through
/// `with_tx` even for single-statement updates so that future
/// invariants — say "every memory created via MCP also gets a
/// default `memory_link`" or "every decision logs an audit-trail
/// row" — can land without changing the public function signature
/// of each writer.
///
/// The helper acquires the connection mutex once; the transaction
/// is dropped (rolled back) on the early-return path if `f`
/// returns `Err`, or committed at the end if `f` returns `Ok`.
fn with_tx<F, T>(idx: &SessionIndex, f: F) -> Result<T, DurableError>
where
    F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T, DurableError>,
{
    let db = idx.db();
    let tx = db.unchecked_transaction()?;
    let result = f(&tx)?;
    tx.commit()?;
    Ok(result)
}

// ─── memories ─────────────────────────────────────────────────

pub fn create_memory(
    idx: &SessionIndex,
    new: &NewMemory<'_>,
) -> Result<MemoryRecord, DurableError> {
    // Pre-flight scope check; the DB CHECK constraint also enforces.
    if matches!(new.scope, Scope::Global) && new.project_path.is_some() {
        return Err(DurableError::InvalidScope {
            scope: new.scope,
            project_path: new.project_path.map(String::from),
        });
    }
    if matches!(new.scope, Scope::Project) && new.project_path.is_none() {
        return Err(DurableError::InvalidScope {
            scope: new.scope,
            project_path: None,
        });
    }

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    with_tx(idx, |tx| {
        tx.execute(
            "INSERT INTO memories (
                id, scope, project_path, kind, content,
                created_by_kind, created_by,
                confidence, created_at_ms, updated_at_ms, archived_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, NULL)",
            params![
                id,
                new.scope.as_str(),
                new.project_path,
                new.kind.as_str(),
                new.content,
                new.created_by_kind.as_str(),
                new.created_by,
                new.confidence,
                now,
            ],
        )?;
        Ok(())
    })?;
    Ok(MemoryRecord {
        id,
        scope: new.scope,
        project_path: new.project_path.map(String::from),
        kind: new.kind,
        content: new.content.to_string(),
        created_by_kind: new.created_by_kind,
        created_by: new.created_by.to_string(),
        confidence: new.confidence,
        created_at_ms: now,
        updated_at_ms: now,
        archived_at_ms: None,
    })
}

pub fn archive_memory(idx: &SessionIndex, id: &str) -> Result<bool, DurableError> {
    let now = chrono::Utc::now().timestamp_millis();
    with_tx(idx, |tx| {
        let n = tx.execute(
            "UPDATE memories SET archived_at_ms = ?1 WHERE id = ?2 AND archived_at_ms IS NULL",
            params![now, id],
        )?;
        Ok(n > 0)
    })
}

#[derive(Debug, Clone, Default)]
pub struct MemoryListFilter {
    pub scope: Option<Scope>,
    pub project_path: Option<String>,
    pub kind: Option<MemoryKind>,
    /// Include archived rows? Default `false`.
    pub include_archived: bool,
    pub limit: u32,
}

pub fn list_memories(
    idx: &SessionIndex,
    f: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>, DurableError> {
    let limit = if f.limit == 0 { 100 } else { f.limit.min(500) };
    let mut sql = String::from(
        "SELECT id, scope, project_path, kind, content, created_by_kind, created_by, \
                confidence, created_at_ms, updated_at_ms, archived_at_ms \
         FROM memories WHERE 1=1",
    );
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();
    let mut nxt = 1;
    if let Some(scope) = f.scope {
        sql.push_str(&format!(" AND scope = ?{}", nxt));
        binds.push(rusqlite::types::Value::Text(scope.as_str().to_string()));
        nxt += 1;
    }
    if let Some(ref pp) = f.project_path {
        sql.push_str(&format!(" AND project_path = ?{}", nxt));
        binds.push(rusqlite::types::Value::Text(pp.clone()));
        nxt += 1;
    }
    if let Some(kind) = f.kind {
        sql.push_str(&format!(" AND kind = ?{}", nxt));
        binds.push(rusqlite::types::Value::Text(kind.as_str().to_string()));
        nxt += 1;
    }
    if !f.include_archived {
        sql.push_str(" AND archived_at_ms IS NULL");
    }
    sql.push_str(&format!(" ORDER BY updated_at_ms DESC LIMIT ?{}", nxt));
    binds.push(rusqlite::types::Value::Integer(limit as i64));

    let db = idx.db();
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |row| {
        Ok(MemoryRecord {
            id: row.get(0)?,
            scope: scope_from_str(&row.get::<_, String>(1)?),
            project_path: row.get(2)?,
            kind: memory_kind_from_str(&row.get::<_, String>(3)?),
            content: row.get(4)?,
            created_by_kind: created_by_kind_from_str(&row.get::<_, String>(5)?),
            created_by: row.get(6)?,
            confidence: row.get(7)?,
            created_at_ms: row.get(8)?,
            updated_at_ms: row.get(9)?,
            archived_at_ms: row.get(10)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ─── decisions ────────────────────────────────────────────────

pub fn log_decision(
    idx: &SessionIndex,
    new: &NewDecision<'_>,
) -> Result<DecisionRecord, DurableError> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    with_tx(idx, |tx| {
        tx.execute(
            "INSERT INTO decisions (
                id, project_path, topic, decision, rationale,
                status, created_by_kind, created_by, created_at_ms, supersedes_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, NULL)",
            params![
                id,
                new.project_path,
                new.topic,
                new.decision,
                new.rationale,
                new.created_by_kind.as_str(),
                new.created_by,
                now,
            ],
        )?;
        Ok(())
    })?;
    Ok(DecisionRecord {
        id,
        project_path: new.project_path.map(String::from),
        topic: new.topic.map(String::from),
        decision: new.decision.to_string(),
        rationale: new.rationale.map(String::from),
        status: DecisionStatus::Active,
        created_by_kind: new.created_by_kind,
        created_by: new.created_by.to_string(),
        created_at_ms: now,
        supersedes_id: None,
    })
}

/// Transition an active decision to `archived`. Use this for
/// decisions that are no longer in force but weren't replaced by a
/// specific successor (use `supersede_decision` when there's a
/// replacement). Returns `true` if the row transitioned, `false`
/// if no active decision with that id existed.
///
/// M13 — closes the API asymmetry where the schema CHECK and the
/// `DecisionStatus::Archived` enum branch existed but no function
/// produced the state.
pub fn archive_decision(idx: &SessionIndex, id: &str) -> Result<bool, DurableError> {
    with_tx(idx, |tx| {
        let n = tx.execute(
            "UPDATE decisions SET status = 'archived' WHERE id = ?1 AND status = 'active'",
            [id],
        )?;
        Ok(n > 0)
    })
}

/// Create a new active decision that supersedes an existing one.
/// The prior decision flips to `superseded`. Atomic.
pub fn supersede_decision(
    idx: &SessionIndex,
    prior_id: &str,
    new: &NewDecision<'_>,
) -> Result<DecisionRecord, DurableError> {
    let now = chrono::Utc::now().timestamp_millis();
    let id = Uuid::new_v4().to_string();
    with_tx(idx, |tx| {
        let n = tx.execute(
            "UPDATE decisions SET status = 'superseded' WHERE id = ?1 AND status = 'active'",
            [prior_id],
        )?;
        if n == 0 {
            return Err(DurableError::DecisionNotFound(prior_id.to_string()));
        }
        tx.execute(
            "INSERT INTO decisions (
                id, project_path, topic, decision, rationale,
                status, created_by_kind, created_by, created_at_ms, supersedes_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9)",
            params![
                id,
                new.project_path,
                new.topic,
                new.decision,
                new.rationale,
                new.created_by_kind.as_str(),
                new.created_by,
                now,
                prior_id,
            ],
        )?;
        Ok(())
    })?;
    Ok(DecisionRecord {
        id,
        project_path: new.project_path.map(String::from),
        topic: new.topic.map(String::from),
        decision: new.decision.to_string(),
        rationale: new.rationale.map(String::from),
        status: DecisionStatus::Active,
        created_by_kind: new.created_by_kind,
        created_by: new.created_by.to_string(),
        created_at_ms: now,
        supersedes_id: Some(prior_id.to_string()),
    })
}

#[derive(Debug, Clone, Default)]
pub struct DecisionListFilter {
    pub project_path: Option<String>,
    pub status: Option<DecisionStatus>,
    pub limit: u32,
}

pub fn list_decisions(
    idx: &SessionIndex,
    f: &DecisionListFilter,
) -> Result<Vec<DecisionRecord>, DurableError> {
    let limit = if f.limit == 0 { 100 } else { f.limit.min(500) };
    let mut sql = String::from(
        "SELECT id, project_path, topic, decision, rationale, status, \
                created_by_kind, created_by, created_at_ms, supersedes_id \
         FROM decisions WHERE 1=1",
    );
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();
    let mut nxt = 1;
    if let Some(ref pp) = f.project_path {
        sql.push_str(&format!(" AND project_path = ?{}", nxt));
        binds.push(rusqlite::types::Value::Text(pp.clone()));
        nxt += 1;
    }
    if let Some(status) = f.status {
        sql.push_str(&format!(" AND status = ?{}", nxt));
        binds.push(rusqlite::types::Value::Text(status.as_str().to_string()));
        nxt += 1;
    }
    sql.push_str(&format!(" ORDER BY created_at_ms DESC LIMIT ?{}", nxt));
    binds.push(rusqlite::types::Value::Integer(limit as i64));

    let db = idx.db();
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |row| {
        Ok(DecisionRecord {
            id: row.get(0)?,
            project_path: row.get(1)?,
            topic: row.get(2)?,
            decision: row.get(3)?,
            rationale: row.get(4)?,
            status: decision_status_from_str(&row.get::<_, String>(5)?),
            created_by_kind: created_by_kind_from_str(&row.get::<_, String>(6)?),
            created_by: row.get(7)?,
            created_at_ms: row.get(8)?,
            supersedes_id: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ─── evidence ─────────────────────────────────────────────────

pub fn submit_evidence(
    idx: &SessionIndex,
    new: &NewEvidence<'_>,
) -> Result<EvidenceRecord, DurableError> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    with_tx(idx, |tx| {
        tx.execute(
            "INSERT INTO evidence_records (
                id, project_path, topic, summary, verification,
                files_changed_json, confidence,
                created_by_kind, created_by, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                new.project_path,
                new.topic,
                new.summary,
                new.verification,
                new.files_changed_json,
                new.confidence,
                new.created_by_kind.as_str(),
                new.created_by,
                now,
            ],
        )?;
        Ok(())
    })?;
    Ok(EvidenceRecord {
        id,
        project_path: new.project_path.map(String::from),
        topic: new.topic.map(String::from),
        summary: new.summary.to_string(),
        verification: new.verification.to_string(),
        files_changed_json: new.files_changed_json.to_string(),
        confidence: new.confidence,
        created_by_kind: new.created_by_kind,
        created_by: new.created_by.to_string(),
        created_at_ms: now,
    })
}

pub fn list_evidence(
    idx: &SessionIndex,
    project_path: Option<&str>,
    limit: u32,
) -> Result<Vec<EvidenceRecord>, DurableError> {
    let lim = if limit == 0 { 100 } else { limit.min(500) };
    let db = idx.db();
    let (sql, params): (&str, Vec<rusqlite::types::Value>) = if let Some(pp) = project_path {
        (
            "SELECT id, project_path, topic, summary, verification, files_changed_json, \
                    confidence, created_by_kind, created_by, created_at_ms \
             FROM evidence_records WHERE project_path = ?1 \
             ORDER BY created_at_ms DESC LIMIT ?2",
            vec![
                rusqlite::types::Value::Text(pp.to_string()),
                rusqlite::types::Value::Integer(lim as i64),
            ],
        )
    } else {
        (
            "SELECT id, project_path, topic, summary, verification, files_changed_json, \
                    confidence, created_by_kind, created_by, created_at_ms \
             FROM evidence_records ORDER BY created_at_ms DESC LIMIT ?1",
            vec![rusqlite::types::Value::Integer(lim as i64)],
        )
    };
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(EvidenceRecord {
            id: row.get(0)?,
            project_path: row.get(1)?,
            topic: row.get(2)?,
            summary: row.get(3)?,
            verification: row.get(4)?,
            files_changed_json: row.get(5)?,
            confidence: row.get(6)?,
            created_by_kind: created_by_kind_from_str(&row.get::<_, String>(7)?),
            created_by: row.get(8)?,
            created_at_ms: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ─── memory links ─────────────────────────────────────────────

/// Arguments to `link`. The caller picks exactly one parent and
/// exactly one target; the DB's CHECK constraints enforce.
#[derive(Debug, Clone)]
pub struct NewLink<'a> {
    /// Pick one: parent.
    pub parent: LinkParent<'a>,
    /// Pick one: target.
    pub target: LinkTarget<'a>,
    pub relation: LinkRelation,
}

#[derive(Debug, Clone)]
pub enum LinkParent<'a> {
    Memory(&'a str),
    Decision(&'a str),
    Evidence(&'a str),
}

#[derive(Debug, Clone)]
pub enum LinkTarget<'a> {
    Exchange(&'a str),
    File(&'a str),
}

pub fn link(idx: &SessionIndex, l: &NewLink<'_>) -> Result<MemoryLinkRecord, DurableError> {
    let id = Uuid::new_v4().to_string();
    let (mem, dec, ev) = match l.parent {
        LinkParent::Memory(s) => (Some(s.to_string()), None, None),
        LinkParent::Decision(s) => (None, Some(s.to_string()), None),
        LinkParent::Evidence(s) => (None, None, Some(s.to_string())),
    };
    let (ex, fp) = match l.target {
        LinkTarget::Exchange(s) => (Some(s.to_string()), None),
        LinkTarget::File(s) => (None, Some(s.to_string())),
    };
    with_tx(idx, |tx| {
        tx.execute(
            "INSERT INTO memory_links (id, memory_id, decision_id, evidence_id, exchange_id, file_path, relation) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, mem.as_deref(), dec.as_deref(), ev.as_deref(), ex.as_deref(), fp.as_deref(), l.relation.as_str()],
        )?;
        Ok(())
    })?;
    Ok(MemoryLinkRecord {
        id,
        memory_id: mem,
        decision_id: dec,
        evidence_id: ev,
        exchange_id: ex,
        file_path: fp,
        relation: l.relation,
    })
}

// ─── enum mappers ─────────────────────────────────────────────

fn scope_from_str(s: &str) -> Scope {
    if s == "global" {
        Scope::Global
    } else {
        Scope::Project
    }
}

fn memory_kind_from_str(s: &str) -> MemoryKind {
    match s {
        "preference" => MemoryKind::Preference,
        "pattern" => MemoryKind::Pattern,
        "constraint" => MemoryKind::Constraint,
        "summary" => MemoryKind::Summary,
        _ => MemoryKind::Fact,
    }
}

fn created_by_kind_from_str(s: &str) -> CreatedByKind {
    match s {
        "agent" => CreatedByKind::Agent,
        "import" => CreatedByKind::Import,
        "system" => CreatedByKind::System,
        _ => CreatedByKind::User,
    }
}

fn decision_status_from_str(s: &str) -> DecisionStatus {
    match s {
        "superseded" => DecisionStatus::Superseded,
        "archived" => DecisionStatus::Archived,
        _ => DecisionStatus::Active,
    }
}

// ─── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_idx(tmp: &TempDir) -> SessionIndex {
        SessionIndex::open(&tmp.path().join("sessions.db")).unwrap()
    }

    // ─── memories ──────────────────────────────────────────────

    #[test]
    fn create_and_list_global_memory() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Global,
                project_path: None,
                kind: MemoryKind::Fact,
                content: "sky is blue",
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
                confidence: Some(95),
            },
        )
        .unwrap();
        assert_eq!(m.scope, Scope::Global);
        assert_eq!(m.kind, MemoryKind::Fact);
        let listed = list_memories(&idx, &MemoryListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, m.id);
    }

    #[test]
    fn invalid_scope_combo_rejected_at_application_layer() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let err = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Global,
                project_path: Some("/proj"),
                kind: MemoryKind::Fact,
                content: "bad",
                created_by_kind: CreatedByKind::User,
                created_by: "u",
                confidence: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, DurableError::InvalidScope { .. }),
            "expected InvalidScope, got {err:?}"
        );
    }

    #[test]
    fn archive_memory_hides_from_default_list() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some("/p"),
                kind: MemoryKind::Preference,
                content: "use rmcp",
                created_by_kind: CreatedByKind::Agent,
                created_by: "codex@test",
                confidence: None,
            },
        )
        .unwrap();
        assert!(archive_memory(&idx, &m.id).unwrap());
        let default = list_memories(&idx, &MemoryListFilter::default()).unwrap();
        assert!(default.is_empty(), "archived row hidden by default");
        let with_archived = list_memories(
            &idx,
            &MemoryListFilter {
                include_archived: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(with_archived.len(), 1);
    }

    #[test]
    fn agent_authorship_recorded() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Global,
                project_path: None,
                kind: MemoryKind::Fact,
                content: "x",
                created_by_kind: CreatedByKind::Agent,
                created_by: "codex@2026-05-15",
                confidence: None,
            },
        )
        .unwrap();
        assert_eq!(m.created_by_kind, CreatedByKind::Agent);
        assert_eq!(m.created_by, "codex@2026-05-15");
    }

    // ─── decisions ────────────────────────────────────────────

    #[test]
    fn log_then_supersede_decision() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let d1 = log_decision(
            &idx,
            &NewDecision {
                project_path: Some("/p"),
                topic: Some("storage"),
                decision: "use SQLite",
                rationale: Some("local-first"),
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
            },
        )
        .unwrap();
        assert_eq!(d1.status, DecisionStatus::Active);

        let d2 = supersede_decision(
            &idx,
            &d1.id,
            &NewDecision {
                project_path: Some("/p"),
                topic: Some("storage"),
                decision: "use SQLite + FTS5",
                rationale: Some("search came up"),
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
            },
        )
        .unwrap();
        assert_eq!(d2.supersedes_id.as_deref(), Some(d1.id.as_str()));
        assert_eq!(d2.status, DecisionStatus::Active);

        // Listing without status filter returns both, newest first.
        let all = list_decisions(&idx, &DecisionListFilter::default()).unwrap();
        assert_eq!(all.len(), 2);

        // Filter for active should only return the new one.
        let active = list_decisions(
            &idx,
            &DecisionListFilter {
                status: Some(DecisionStatus::Active),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, d2.id);

        // The superseded one is queryable separately.
        let superseded = list_decisions(
            &idx,
            &DecisionListFilter {
                status: Some(DecisionStatus::Superseded),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(superseded.len(), 1);
        assert_eq!(superseded[0].id, d1.id);
    }

    #[test]
    fn archive_decision_flips_status() {
        // M13 — archive_decision closes the API asymmetry. A decision
        // that's been replaced informally (not via supersede_decision)
        // can now be marked archived; future list queries can filter
        // it out via status.
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let d = log_decision(
            &idx,
            &NewDecision {
                project_path: Some("/p"),
                topic: Some("storage"),
                decision: "use SQLite",
                rationale: None,
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
            },
        )
        .unwrap();
        assert_eq!(d.status, DecisionStatus::Active);

        assert!(archive_decision(&idx, &d.id).unwrap());

        let archived = list_decisions(
            &idx,
            &DecisionListFilter {
                status: Some(DecisionStatus::Archived),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, d.id);
        // Re-archive is a no-op (status no longer 'active' → 0 rows).
        assert!(!archive_decision(&idx, &d.id).unwrap());
        // Archive non-existent id → false (no row to flip).
        assert!(!archive_decision(&idx, "nope").unwrap());
    }

    #[test]
    fn supersede_nonexistent_decision_errors() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let err = supersede_decision(
            &idx,
            "no-such-id",
            &NewDecision {
                project_path: None,
                topic: None,
                decision: "x",
                rationale: None,
                created_by_kind: CreatedByKind::User,
                created_by: "u",
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, DurableError::DecisionNotFound(_)),
            "expected DecisionNotFound, got {err:?}"
        );
    }

    // ─── evidence ─────────────────────────────────────────────

    #[test]
    fn submit_and_list_evidence() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        let e = submit_evidence(
            &idx,
            &NewEvidence {
                project_path: Some("/p"),
                topic: Some("audit-fix"),
                summary: "3 issues found and fixed",
                verification: "cargo test green",
                files_changed_json: r#"["src/a.rs","src/b.rs"]"#,
                confidence: 90,
                created_by_kind: CreatedByKind::Agent,
                created_by: "codex@2026-05-15",
            },
        )
        .unwrap();
        assert_eq!(e.confidence, 90);
        let all = list_evidence(&idx, None, 0).unwrap();
        assert_eq!(all.len(), 1);
        let scoped = list_evidence(&idx, Some("/p"), 0).unwrap();
        assert_eq!(scoped.len(), 1);
        let other = list_evidence(&idx, Some("/elsewhere"), 0).unwrap();
        assert!(other.is_empty());
    }

    // ─── links ────────────────────────────────────────────────

    #[test]
    fn link_memory_to_file_with_relation() {
        let tmp = TempDir::new().unwrap();
        let idx = open_idx(&tmp);
        // Seed a sessions row so file_path target validates.
        {
            let db = idx.db();
            db.execute(
                "INSERT INTO sessions (
                    file_path, slug, session_id,
                    file_size_bytes, file_mtime_ns, file_inode,
                    project_path, project_from_transcript,
                    first_ts_ms, last_ts_ms,
                    event_count, message_count, user_message_count, assistant_message_count,
                    first_user_prompt, models_json,
                    tokens_input, tokens_output, tokens_cache_creation, tokens_cache_read,
                    git_branch, cc_version, display_slug, has_error, is_sidechain,
                    indexed_at_ms, source_kind
                ) VALUES (
                    '/f.jsonl', 's', 'sid',
                    1, 1, 0,
                    '/p', 1,
                    NULL, NULL,
                    0, 0, 0, 0,
                    NULL, '[]',
                    0, 0, 0, 0,
                    NULL, NULL, NULL, 0, 0,
                    1, 'codex'
                )",
                [],
            )
            .unwrap();
        }
        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Global,
                project_path: None,
                kind: MemoryKind::Fact,
                content: "x",
                created_by_kind: CreatedByKind::User,
                created_by: "u",
                confidence: None,
            },
        )
        .unwrap();
        let l = link(
            &idx,
            &NewLink {
                parent: LinkParent::Memory(&m.id),
                target: LinkTarget::File("/f.jsonl"),
                relation: LinkRelation::Origin,
            },
        )
        .unwrap();
        assert_eq!(l.relation, LinkRelation::Origin);
        assert_eq!(l.memory_id.as_deref(), Some(m.id.as_str()));
        assert_eq!(l.file_path.as_deref(), Some("/f.jsonl"));
    }
}
