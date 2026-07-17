//! Run the knowledge distiller over one transcript and file what it
//! finds as review proposals.
//!
//! This is the model half of the harvester: spawn `claude -p` with the
//! distiller prompt, hand its stdout to [`super::proposal`]'s parser,
//! and ingest whatever survives the filters. The deterministic
//! parse/filter/ingest half stays in [`super::proposal`] — this module
//! owns only the subprocess. Callers run in blocking contexts (sync
//! CLI handlers), so this is a synchronous subprocess on purpose —
//! same rationale as [`super::git`].

use crate::agent::templates::{KNOWLEDGE_DISTILLER_MODEL, KNOWLEDGE_DISTILLER_PROMPT};
use crate::session_index::SessionIndex;
use crate::shared_memory::durable::DurableError;
use crate::shared_memory::proposal::{self, IngestReport, ProposalOrigin};

#[derive(Debug, thiserror::Error)]
pub enum DistillError {
    #[error("spawn `claude -p` for the distiller")]
    Spawn(#[source] std::io::Error),

    #[error("claude -p exited {status}: {stderr}")]
    ClaudeFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },

    #[error("parse the distiller's output")]
    Parse(#[source] serde_json::Error),

    #[error(transparent)]
    Ingest(#[from] DurableError),
}

/// Run the distiller over one transcript and file whatever it finds.
///
/// `claude_bin` is the `claude` binary to spawn (name or path — the
/// caller decides how it resolves); `created_by` is stamped onto every
/// filed proposal so the audit trail names the caller. The ingest
/// timestamp is sampled AFTER the subprocess returns, so a minutes-long
/// distillation stamps its proposals when they are filed, not when the
/// run began.
pub fn distill_transcript(
    idx: &SessionIndex,
    claude_bin: &str,
    project: &str,
    transcript: &str,
    created_by: &str,
) -> Result<IngestReport, DistillError> {
    let out = std::process::Command::new(claude_bin)
        .arg("-p")
        .arg(format!(
            "{KNOWLEDGE_DISTILLER_PROMPT}\n\nThe transcript is at: {transcript}\n\n\
             Output ONLY a JSON object of the form {{\"claims\":[...]}}. No prose."
        ))
        .args(["--model", KNOWLEDGE_DISTILLER_MODEL])
        .args(["--allowedTools", "Read,Grep"])
        .env("CLAUDEPOT_EVENT_SESSION_PATH", transcript)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(DistillError::Spawn)?;
    if !out.status.success() {
        return Err(DistillError::ClaudeFailed {
            status: out.status,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let claims = proposal::parse_claims(&raw).map_err(DistillError::Parse)?;

    let origin = ProposalOrigin {
        project_path: project,
        file_path: Some(transcript),
        exchange_id: None,
        created_by,
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(proposal::ingest_proposals(idx, &claims, &origin, now_ms)?)
}
