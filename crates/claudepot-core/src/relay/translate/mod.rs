//! The `Translated` upstream: Anthropic Messages in, a provider's
//! Responses or Chat Completions protocol out, and back (plan §9, P10).
//!
//! Ported from clai's `clcd` filters, case by case, against the fixtures
//! in `testdata/translate/` — never from memory. The pipeline per request:
//!
//! ```text
//! Anthropic request ─ request::to_responses ─▶ Responses request
//!                                  (chat, gemini) ─ chat::to_chat ─▶ Chat request
//! provider SSE ─ stream::SseLines ─▶ (chat::ChatToResponses ─▶) Responses events
//!              ─ stream::ResponsesToAnthropic ─▶ Anthropic SSE
//! ```
//!
//! ## Capability contract
//!
//! What every translated backend **preserves**: text, images in user
//! turns, tool definitions and calls (names round-trip through
//! [`request::Names`]), tool results including errors, system prompts,
//! usage (input, output, cached input).
//!
//! What it **converts**: `thinking` / `redacted_thinking` blocks are
//! dropped (reviewed exception E1 — no provider accepts Anthropic's
//! signed thinking); images inside a `tool_result` are lifted into a user
//! message (E3); `incomplete` with `content_filter` becomes
//! `stop_reason: "refusal"`, any other `incomplete` `max_tokens` (E2).
//!
//! What it **refuses before dispatch**, as `400` with a fixed code:
//! `count_tokens` (`count_tokens_unsupported`, decided in P-D — never an
//! estimate presented as the provider's count, E8); content the backend
//! cannot carry (`content_unsupported`: images outside a user message on
//! Chat, `disable_parallel_tool_use` on Chat, reasoning effort on Grok and
//! Chat — E10, E12); and a Gemini tool result whose call's thought
//! signature is not in [`continuation::ContinuationStore`]
//! (`continuation_state_lost`, Q4).
//!
//! Upstream text never reaches the client (E5, E6): a provider error is
//! re-stated by [`errors`] with fixed text; a stream fault by
//! [`stream::Fault`].
//!
//! ## Bounds
//!
//! Every buffer an adapter keeps has a cap below; crossing one fails that
//! request with `Fault::Oversize` and nothing else.

pub mod chat;
pub mod codex;
pub mod continuation;
pub mod errors;
pub mod handler;
pub mod request;
pub mod stream;

use sha2::{Digest, Sha256};

use super::identity::{CredentialIdentity, TargetKind, TargetSpec};
use super::route_secret::RouteSecretError;
use crate::routes::{Route, RouteProvider};

/// One SSE line.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Content blocks in one response.
pub const MAX_BLOCKS: usize = 1024;
/// Tool calls in one Chat response.
pub const MAX_CALLS: usize = 128;
/// One tool call's argument buffer.
pub const MAX_ARGS_PER_CALL: usize = 1024 * 1024;
/// All tool calls' argument buffers together.
pub const MAX_ARGS_TOTAL: usize = 8 * 1024 * 1024;
/// One Chat tool call's id, and separately its name. Both are replaced,
/// not appended, so this bounds what [`MAX_CALLS`] calls retain.
pub const MAX_CALL_FIELD: usize = 1024;
/// One call's provider metadata (`extra_content`).
pub const MAX_EXTRA_CONTENT: usize = 64 * 1024;
/// Text retained to build a non-streaming response.
pub const MAX_RETAINED_TEXT: usize = 8 * 1024 * 1024;

/// The wire protocol a translated route speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Translation {
    /// ChatGPT's Codex Responses endpoint, on the relay's own Codex login.
    Codex,
    /// OpenAI's Responses API, on the route's key.
    Openai,
    /// xAI's Responses API, on the route's key.
    Grok,
    /// Gemini's OpenAI-compatible Chat Completions, with thought signatures.
    Gemini,
    /// Any other Chat Completions provider.
    Chat,
}

impl Translation {
    pub const ALL: [Translation; 5] = [
        Translation::Codex,
        Translation::Openai,
        Translation::Grok,
        Translation::Gemini,
        Translation::Chat,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Translation::Codex => "codex",
            Translation::Openai => "openai",
            Translation::Grok => "grok",
            Translation::Gemini => "gemini",
            Translation::Chat => "chat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Whether the provider speaks Chat Completions (translated through
    /// the Responses shape) rather than Responses.
    pub fn is_chat(self) -> bool {
        matches!(self, Translation::Gemini | Translation::Chat)
    }

    /// Whether the credential is the route's key. Codex's is the relay's
    /// own Codex login ([`codex`]).
    pub fn uses_route_key(self) -> bool {
        self != Translation::Codex
    }
}

/// The only origin a Codex route may name: the Codex backend the login is
/// for. Anything else would send a ChatGPT token somewhere it does not
/// belong.
pub const CODEX_ORIGIN: &str = "https://chatgpt.com/backend-api/codex";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TranslateRouteError {
    #[error(transparent)]
    Secret(#[from] RouteSecretError),
    #[error("a codex route must use the base URL {CODEX_ORIGIN}")]
    CodexOrigin,
    #[error("the route's base URL must be https (or http on loopback)")]
    Insecure,
}

fn origin_of(route: &Route) -> Result<String, TranslateRouteError> {
    let RouteProvider::Gateway(cfg) = &route.provider else {
        return Err(RouteSecretError::NotRelayCapable(route.id).into());
    };
    Ok(cfg.base_url.trim().trim_end_matches('/').to_owned())
}

/// The URL requests go to.
///
/// A gateway route is saved with a bare trailing `/v1` stripped
/// ([`crate::routes::normalize_gateway_base_url`]), because CC appends
/// `/v1/messages` itself on the pass-through path. A translated provider's
/// API lives under `/v1` too, so an origin with no path at all gets it
/// back: the `https://api.openai.com/v1` the user saved is stored as
/// `https://api.openai.com` and must still reach `/v1/responses`. An
/// origin that has a path (`/v1beta/openai`, `/api/v1`, Codex's
/// `/backend-api/codex`) is used as written.
pub fn endpoint(origin: &str, translation: Translation) -> String {
    let suffix = if translation.is_chat() {
        "/chat/completions"
    } else {
        "/responses"
    };
    if origin.ends_with(suffix) {
        return origin.to_owned();
    }
    let bare = reqwest::Url::parse(origin)
        .is_ok_and(|u| u.path() == "/" && u.query().is_none() && u.fragment().is_none());
    if bare {
        format!("{}/v1{suffix}", origin.trim_end_matches('/'))
    } else {
        format!("{origin}{suffix}")
    }
}

/// Whether requests to `origin` keep the route's credential private:
/// `https`, or `http` to this machine. The URL is parsed, never split by
/// hand — `http://localhost:80@example.invalid` names `example.invalid`,
/// with `localhost:80` as userinfo — and any userinfo is refused, because
/// reqwest would send it as credentials and it only ever disguises a host.
fn is_secure_origin(origin: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(origin) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some_and(|h| !h.is_empty()),
        "http" => is_loopback_host(&url),
        _ => false,
    }
}

/// `localhost`, `127.0.0.0/8` or `::1`, as the parser resolved the host.
fn is_loopback_host(url: &reqwest::Url) -> bool {
    if let Some(domain) = url.domain() {
        return domain == "localhost";
    }
    url.host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
        .and_then(|h| h.parse::<std::net::IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback())
}

/// A translated route's target: kind, origin, protocol, the model mapping
/// and who pays. A Codex target pays with the relay's Codex login, named
/// by a fixed id, so a re-login keeps the target; a keyed target follows
/// the key's identity exactly as a pass-through route does.
pub fn route_target_spec(
    route: &Route,
    translation: Translation,
) -> Result<TargetSpec, TranslateRouteError> {
    let origin = origin_of(route)?;
    if translation == Translation::Codex && origin != CODEX_ORIGIN {
        return Err(TranslateRouteError::CodexOrigin);
    }
    if !is_secure_origin(&origin) {
        return Err(TranslateRouteError::Insecure);
    }
    let mut spec = super::passthrough::route_target_spec(route)?;
    spec.kind = TargetKind::Translated;
    spec.origin = origin;
    spec.protocol = Some(translation.as_str().to_owned());
    if !translation.uses_route_key() {
        spec.credential = CredentialIdentity::Key {
            key_id_digest: hex::encode(Sha256::digest(b"codex:relay-codex")),
        };
    }
    Ok(spec)
}

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;
