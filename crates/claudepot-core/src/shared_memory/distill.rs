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

use crate::agent::templates::{
    KNOWLEDGE_DISTILLER_JSON_SCHEMA, KNOWLEDGE_DISTILLER_MODEL, KNOWLEDGE_DISTILLER_PROMPT,
};
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

/// Flags shared with the scheduled Distiller agent.
///
/// `agent::templates::knowledge_distiller` sets `output_format: Json`
/// plus `json_schema`, which `agent::shim::build_cli_flags` renders into
/// `--output-format json --json-schema <schema>`. This path shipped
/// without either: it asked for JSON in prose and hoped. That made it
/// the one caller where `proposal::parse_claims`'s tolerance was
/// load-bearing rather than defense in depth.
///
/// Extracted so the two paths can be asserted equal — see
/// `tests::the_manual_path_uses_the_same_contract_as_the_agent`.
fn distiller_flags() -> Vec<String> {
    vec![
        "--model".into(),
        KNOWLEDGE_DISTILLER_MODEL.into(),
        "--output-format".into(),
        "json".into(),
        "--json-schema".into(),
        KNOWLEDGE_DISTILLER_JSON_SCHEMA.into(),
        "--allowedTools".into(),
        "Read,Grep".into(),
    ]
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
        .args(distiller_flags())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::templates::knowledge_distiller;

    fn flag_value<'a>(flags: &'a [String], name: &str) -> Option<&'a str> {
        let i = flags.iter().position(|f| f == name)?;
        flags.get(i + 1).map(String::as_str)
    }

    /// The two ways a distiller run can happen — the scheduled agent and
    /// this manual path — must ask the model for the same thing. They
    /// had drifted: the agent template set `output_format: Json` and a
    /// `json_schema`, and this path set neither, so the harvest most
    /// likely to be run by hand was the one with no schema enforcing its
    /// shape.
    #[test]
    fn the_manual_path_uses_the_same_contract_as_the_agent() {
        let flags = distiller_flags();
        let agent = knowledge_distiller("/tmp", chrono::Utc::now());

        assert_eq!(
            flag_value(&flags, "--output-format"),
            Some(agent.output_format.as_cli_flag()),
        );
        assert_eq!(
            flag_value(&flags, "--json-schema"),
            agent.json_schema.as_deref(),
        );
        assert_eq!(flag_value(&flags, "--model"), agent.model.as_deref());
        assert_eq!(
            flag_value(&flags, "--allowedTools"),
            Some(agent.allowed_tools.join(",").as_str()),
        );
    }

    /// The schema is sent to CC verbatim; a typo in it would be rejected
    /// at run time, far from here.
    #[test]
    fn the_shipped_schema_is_valid_json() {
        let schema: serde_json::Value =
            serde_json::from_str(KNOWLEDGE_DISTILLER_JSON_SCHEMA).expect("schema parses");
        assert!(schema.pointer("/properties/claims").is_some());
    }
}
