//! Built-in agent templates (PRD §9.2 / D5).
//!
//! v1 ships **exactly one** built-in template — the **Session
//! Narrator**, the agent that motivated the whole Agents line of
//! work. A *catalog* of templates is explicitly v2 (PRD §13); this
//! module is deliberately a single function, not a registry.
//!
//! A template instantiates to a **draft** (`lifecycle = Draft`,
//! `drafted_by = "template:session-narrator"`). The draft is inert
//! until a human reviews and installs it via the existing Phase-2
//! Review & install flow — that click *is* the human-approval gate
//! (PRD §8.2). The template never produces an armed agent.
//!
//! Pure: no I/O, no env reads. The caller supplies the `cwd` (the
//! project the narrator watches) and the clock.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::types::{
    Agent, AgentBinary, CreatedVia, EventKind, Lifecycle, McpServerRef, OutputFormat,
    PermissionMode, PlatformOptions, RateLimit, ResultSink, Trigger, DEFAULT_DEBOUNCE_SECS,
};

/// Stable id recorded in `drafted_by` for Session-Narrator drafts.
/// The audit trail distinguishes a template-drafted agent from an
/// AI-drafted (`claude-code@…`) or hand-created (`None`) one.
pub const SESSION_NARRATOR_DRAFTED_BY: &str = "template:session-narrator";

/// The Session Narrator's `template_id`. Set on the produced agent
/// so template-aware run handling (output-artifact discovery) and
/// the GUI can recognize it.
pub const SESSION_NARRATOR_TEMPLATE_ID: &str = "session-narrator";

/// Default model: summarization is Haiku work, not Opus work
/// (PRD §9.2). A versioned id, never a bare alias.
pub const SESSION_NARRATOR_MODEL: &str = "claude-haiku-4-5";

/// The Narrator's digest prompt. Asks for a readable account of the
/// settled session; the `CLAUDEPOT_EVENT_SESSION_ID` /
/// `CLAUDEPOT_EVENT_SESSION_PATH` env vars the orchestrator injects
/// point the model at the exact transcript.
const SESSION_NARRATOR_PROMPT: &str = "\
Produce a readable, plain-English account of what Claude Code did in \
the most recently settled session. The session id is in the \
CLAUDEPOT_EVENT_SESSION_ID environment variable and the transcript \
file is at CLAUDEPOT_EVENT_SESSION_PATH.

Read the transcript and write a digest that covers, in order:
1. What the user asked for (the goal of the session).
2. What Claude actually did — the files it changed, the commands it \
ran, the decisions it made.
3. The outcome — what shipped, what is still open, any errors.

Keep it concise and skimmable. Lead with the outcome. Do not quote \
long code blocks; describe changes instead. Write for someone who \
was not watching the session.";

/// A clear digest-shaped JSON schema so the run emits structured
/// output the Run-History panel can render as content (PRD §10).
const SESSION_NARRATOR_JSON_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "headline": {
      "type": "string",
      "description": "One-line summary of the session outcome."
    },
    "goal": {
      "type": "string",
      "description": "What the user asked Claude Code to do."
    },
    "actions": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Notable things Claude did, in order."
    },
    "outcome": {
      "type": "string",
      "description": "What shipped and what is still open."
    }
  },
  "required": ["headline", "goal", "actions", "outcome"]
}"#;

/// Build the **Session Narrator** as a pre-filled draft [`Agent`].
///
/// The returned agent has `lifecycle = Draft` and
/// `drafted_by = "template:session-narrator"`; persisting it (via
/// `AgentStore::add` + `save`) is the caller's job, and even a
/// persisted draft stays inert until a human arms it through the
/// GUI Review & install flow.
///
/// `cwd` is the project the narrator watches — the `session-settled`
/// trigger only fires for sessions in that project (PRD §7, scope
/// rule). `now` is injected for testability.
///
/// The draft carries a **mandatory `rate_limit`** because its
/// trigger is an event trigger (PRD D9): without one, an event agent
/// could fire on every settled session unbounded. The cap here —
/// at most one run every 30 minutes, 12 per day — is conservative;
/// the user can loosen it in the Review modal before installing.
pub fn session_narrator(cwd: &str, now: DateTime<Utc>) -> Agent {
    Agent {
        id: Uuid::new_v4(),
        name: "session-narrator".to_string(),
        display_name: Some("Session Narrator".to_string()),
        description: Some(
            "Writes a readable digest of each finished Claude Code \
             session in this project."
                .to_string(),
        ),
        // A draft is created enabled so that, once a human arms it,
        // the install path activates it immediately. `Lifecycle`,
        // not `enabled`, is what keeps it inert until then.
        enabled: true,
        binary: AgentBinary::FirstParty,
        model: Some(SESSION_NARRATOR_MODEL.to_string()),
        cwd: cwd.to_string(),
        prompt: SESSION_NARRATOR_PROMPT.to_string(),
        system_prompt: None,
        append_system_prompt: None,
        // Read-only work — the narrator inspects a transcript and
        // writes a digest. Default permission mode; no elevation.
        permission_mode: PermissionMode::Default,
        allowed_tools: vec!["Read".to_string(), "Grep".to_string()],
        add_dir: Vec::new(),
        max_budget_usd: None,
        fallback_model: None,
        // Structured output so RunHistoryPanel renders the digest as
        // content, not just an exit code (PRD §10).
        output_format: OutputFormat::Json,
        json_schema: Some(SESSION_NARRATOR_JSON_SCHEMA.to_string()),
        bare: false,
        extra_env: BTreeMap::new(),
        // The flagship reactive trigger: fire when a session in this
        // project has been idle for the default debounce window.
        trigger: Trigger::Event {
            event: EventKind::SessionSettled {
                debounce_secs: DEFAULT_DEBOUNCE_SECS,
            },
        },
        platform_options: PlatformOptions::default(),
        log_retention_runs: 50,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
        template_id: Some(SESSION_NARRATOR_TEMPLATE_ID.to_string()),
        disallowed_tools: Vec::new(),
        // Claudepot's own memory server, attached so the narrator can
        // read project context as a data source (PRD §9.2).
        mcp_servers: vec![McpServerRef::ClaudepotMemory],
        run_as: None,
        task_budget: None,
        // Mandatory for event triggers (D9). Conservative default;
        // adjustable in the Review modal before install.
        rate_limit: Some(RateLimit {
            min_interval_secs: Some(30 * 60),
            max_per_day: Some(12),
        }),
        lifecycle: Lifecycle::Draft,
        drafted_by: Some(SESSION_NARRATOR_DRAFTED_BY.to_string()),
        // Stamped here, never overridable by the caller — the
        // install review distinguishes a template-instantiated
        // record from a hand-authored one (grill finding F19).
        created_via: CreatedVia::Template,
        // The Narrator's output is prose for a human to read once. It
        // deposits nothing durable; that is the Distiller's job.
        result_sink: None,
    }
}

// ─── Knowledge Distiller ─────────────────────────────────────────

pub const KNOWLEDGE_DISTILLER_DRAFTED_BY: &str = "template:knowledge-distiller";
pub const KNOWLEDGE_DISTILLER_TEMPLATE_ID: &str = "knowledge-distiller";

/// Extraction is Haiku work. At ~$0.04 a settled session, a year of
/// this costs less than one Opus afternoon.
pub const KNOWLEDGE_DISTILLER_MODEL: &str = "claude-haiku-4-5";

/// The distiller's prompt.
///
/// Every line of this is load-bearing, and most of it is *prohibition*.
/// The published evidence says an LLM asked to "summarize what it
/// learned" produces context files that make agents measurably **worse**
/// (ETH) — because what it produces is overviews, and an overview is
/// pure token cost with no behavioral payload. What changes behavior is
/// a specific, imperative instruction that names a command or a file.
///
/// So the distiller is not a summarizer. It is a **failure miner**. If
/// nothing went wrong in a session, the correct output is an empty list,
/// and the prompt has to say that in as many ways as possible, because
/// the model's every instinct is to be helpful and produce *something*.
pub const KNOWLEDGE_DISTILLER_PROMPT: &str = "\
Read the transcript at CLAUDEPOT_EVENT_SESSION_PATH (session id in \
CLAUDEPOT_EVENT_SESSION_ID) and extract durable LESSONS — things that \
should change how future work in this project is done.

A lesson is admissible ONLY if something actually went wrong and was \
corrected. Look for:
  - an error, failed test, failed build, or crash, and the fix
  - a wrong assumption that got corrected mid-session
  - a landmine found (\"this silently does X\"), and how to avoid it
  - a convention discovered the hard way (\"must call Y before Z\")
  - a review finding, and the rule that prevents it recurring

For each lesson emit:
  - claim: what is true, stated so a stranger can act on it. One or \
two sentences.
  - directive: ONE imperative line for a future agent, naming a \
concrete command, file, or symbol. Example: \"Run scripts/preflight.sh \
before pushing; cargo test alone does not run the grep guards.\" \
NOT: \"Be careful about tests.\"
  - files: the file paths the lesson depends on, so it can be \
invalidated when they change. Omit if the lesson is not about code.
  - evidence: what happened, in one line. The failure, not the fix.
  - confidence: 0-100. Below 60, do not emit it at all.

HARD RULES — violating any of these makes the output worse than \
emitting nothing:

1. NO SUMMARIES. Do not describe what the session was about. Do not \
write an overview of the architecture. Those are pure cost.
2. NO LESSON WITHOUT A FAILURE. \"This project uses React\" is not a \
lesson, it is a fact anyone can see. If nothing broke, return an \
empty list. An empty list is a CORRECT and EXPECTED answer.
3. NEVER COPY TRANSCRIPT TEXT. State the claim in your own words. Do \
not quote code, output, logs, names, addresses, credentials, or \
numbers from the transcript. If a lesson cannot be stated without \
quoting private data, DROP IT.
4. SKIP SENSITIVE SESSIONS. If the transcript is about personal, \
financial, legal, or medical matters rather than software, return an \
empty list immediately and extract nothing.
5. Prefer FEWER, SHARPER lessons. Three real ones beat ten padded \
ones. Zero beats three invented ones.";

/// Schema for the distiller's output. `claims` may be empty — and the
/// description says so explicitly, because a schema that merely
/// *permits* emptiness still reads to a model as a form to fill in.
pub const KNOWLEDGE_DISTILLER_JSON_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "claims": {
      "type": "array",
      "description": "Durable lessons. MAY BE EMPTY — an empty array is the correct answer when nothing went wrong, or when the session is not about software.",
      "items": {
        "type": "object",
        "properties": {
          "claim": {
            "type": "string",
            "description": "What is true, in your own words. Never quoted from the transcript."
          },
          "directive": {
            "type": "string",
            "description": "One imperative line for a future agent, naming a concrete command, file, or symbol."
          },
          "kind": {
            "type": "string",
            "enum": ["pattern", "constraint", "preference", "fact"],
            "description": "constraint = a rule that must hold; pattern = a recurring shape; preference = how this user wants it; fact = a durable truth."
          },
          "files": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Repo-relative paths the lesson depends on. Used to invalidate the lesson when they change."
          },
          "evidence": {
            "type": "string",
            "description": "The failure that justifies this lesson, in one line."
          },
          "confidence": {
            "type": "integer",
            "minimum": 0,
            "maximum": 100
          }
        },
        "required": ["claim", "directive", "kind", "evidence", "confidence"]
      }
    }
  },
  "required": ["claims"]
}"#;

/// Build the **Knowledge Distiller** as a pre-filled draft [`Agent`].
///
/// Fires on the same `session-settled` trigger as the Narrator, but
/// where the Narrator writes prose for a human to read once, the
/// Distiller emits structured claims that Claudepot ingests as
/// **proposals** for the triage queue.
///
/// Two deliberate omissions:
///
/// - **No MCP servers.** The Narrator attaches the memory server so it
///   can read project context. The Distiller must not: if it could
///   *write*, persistence would again depend on the model choosing to
///   call a tool. It emits JSON; [`ResultSink::MemoryProposals`] does
///   the writing. The model cannot decide not to.
/// - **No write tools.** `Read` and `Grep` only. It reads a transcript
///   and returns a value. It has no business touching the repo.
pub fn knowledge_distiller(cwd: &str, now: DateTime<Utc>) -> Agent {
    Agent {
        id: Uuid::new_v4(),
        name: "knowledge-distiller".to_string(),
        display_name: Some("Knowledge Distiller".to_string()),
        description: Some(
            "Mines each finished session for lessons — things that broke and \
             how to stop them breaking again — and files them for your review."
                .to_string(),
        ),
        enabled: true,
        binary: AgentBinary::FirstParty,
        model: Some(KNOWLEDGE_DISTILLER_MODEL.to_string()),
        cwd: cwd.to_string(),
        prompt: KNOWLEDGE_DISTILLER_PROMPT.to_string(),
        system_prompt: None,
        append_system_prompt: None,
        permission_mode: PermissionMode::Default,
        allowed_tools: vec!["Read".to_string(), "Grep".to_string()],
        add_dir: Vec::new(),
        max_budget_usd: None,
        fallback_model: None,
        output_format: OutputFormat::Json,
        json_schema: Some(KNOWLEDGE_DISTILLER_JSON_SCHEMA.to_string()),
        bare: false,
        extra_env: BTreeMap::new(),
        trigger: Trigger::Event {
            event: EventKind::SessionSettled {
                debounce_secs: DEFAULT_DEBOUNCE_SECS,
            },
        },
        platform_options: PlatformOptions::default(),
        log_retention_runs: 50,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
        template_id: Some(KNOWLEDGE_DISTILLER_TEMPLATE_ID.to_string()),
        disallowed_tools: Vec::new(),
        // Deliberately empty — see the doc comment.
        mcp_servers: Vec::new(),
        run_as: None,
        task_budget: None,
        rate_limit: Some(RateLimit {
            min_interval_secs: Some(30 * 60),
            max_per_day: Some(12),
        }),
        lifecycle: Lifecycle::Draft,
        drafted_by: Some(KNOWLEDGE_DISTILLER_DRAFTED_BY.to_string()),
        created_via: CreatedVia::Template,
        // The whole point: the run's JSON lands in `memories` as
        // proposals, deterministically, without the model having to
        // volunteer a tool call.
        result_sink: Some(ResultSink::MemoryProposals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::slug::validate_name;
    use crate::testing::test_cwd;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn session_narrator_builds_a_valid_draft() {
        let cwd = test_cwd();
        let a = session_narrator(&cwd, now());
        // It is a draft — inert until a human installs it.
        assert_eq!(a.lifecycle, Lifecycle::Draft);
        assert_eq!(a.drafted_by.as_deref(), Some(SESSION_NARRATOR_DRAFTED_BY));
        assert_eq!(a.template_id.as_deref(), Some(SESSION_NARRATOR_TEMPLATE_ID));
        // The name passes the same validation the store enforces.
        validate_name(&a.name).expect("narrator name must be valid");
        assert_eq!(a.cwd, cwd);
    }

    #[test]
    fn session_narrator_uses_haiku_and_session_settled_trigger() {
        let a = session_narrator(&test_cwd(), now());
        assert_eq!(a.model.as_deref(), Some(SESSION_NARRATOR_MODEL));
        match &a.trigger {
            Trigger::Event {
                event: EventKind::SessionSettled { debounce_secs },
            } => assert_eq!(*debounce_secs, DEFAULT_DEBOUNCE_SECS),
            other => panic!("expected SessionSettled trigger, got {other:?}"),
        }
    }

    #[test]
    fn session_narrator_carries_a_rate_limit() {
        // An event-triggered agent MUST carry a rate limit (D9).
        let a = session_narrator(&test_cwd(), now());
        let rl = a.rate_limit.expect("event agent must carry a rate_limit");
        assert!(rl.min_interval_secs.is_some());
        assert!(rl.max_per_day.is_some());
    }

    #[test]
    fn session_narrator_attaches_claudepot_memory() {
        let a = session_narrator(&test_cwd(), now());
        assert!(a
            .mcp_servers
            .iter()
            .any(|m| matches!(m, McpServerRef::ClaudepotMemory)));
    }

    #[test]
    fn session_narrator_emits_structured_output() {
        // The digest must be structured so the Run-History panel can
        // render it as content (PRD §10).
        let a = session_narrator(&test_cwd(), now());
        assert_eq!(a.output_format, OutputFormat::Json);
        assert!(a.json_schema.is_some());
        // The schema is itself valid JSON.
        let schema: serde_json::Value =
            serde_json::from_str(a.json_schema.as_deref().unwrap()).unwrap();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn session_narrator_stamps_template_provenance() {
        // F21 wiring: the template-instantiate path is the only
        // call site that produces this constructor's output, so it
        // is responsible for stamping `created_via = Template`.
        // The GUI install review uses this signal to flag the
        // record as non-hand-authored.
        let a = session_narrator(&test_cwd(), now());
        assert_eq!(a.created_via, CreatedVia::Template);
    }

    #[test]
    fn session_narrator_passes_store_validation() {
        // F21 regression: the produced draft must be accepted by
        // `AgentStore::add` so the (new) `agent_add_from_template`
        // Tauri command actually persists. This pins the contract:
        // every constraint added to `add` (event-trigger rate
        // limit, debounce-secs ceiling, cwd shape, …) must let
        // this template through. If a future rule grows tighter
        // than the template, the test breaks and forces the
        // template's defaults to be adjusted in lock step.
        use super::super::draft::{
            validate_cwd, validate_event_trigger_numerics, validate_rate_limit_numerics,
            validate_trigger_timezone,
        };
        use super::super::slug::validate_name;

        let a = session_narrator(&test_cwd(), now());
        validate_name(&a.name).expect("name");
        validate_cwd(&a.cwd).expect("cwd");
        validate_trigger_timezone(&a.trigger).expect("tz");
        validate_event_trigger_numerics(&a.trigger).expect("debounce");
        validate_rate_limit_numerics(a.rate_limit.as_ref()).expect("rate limit");
    }

    #[test]
    fn session_narrator_round_trips_through_serde() {
        // The produced draft must survive a store write/read cycle.
        let a = session_narrator(&test_cwd(), now());
        let s = serde_json::to_string(&a).unwrap();
        let back: Agent = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
