//! The request-handler boundary every upstream implements.
//!
//! The abstraction sits at the handler, not at "token source → header
//! rewrite": that altitude cannot express capability refusal,
//! continuation state, or a translated terminal error. P1 ships one
//! implementation, [`crate::relay::forward::SubscriptionHandler`]. The
//! trait's shape is provisional until the P-D discovery gate closes —
//! the translate handler (P10) is what will test it.

use std::collections::BTreeSet;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use futures_util::stream::BoxStream;
use tokio::sync::{oneshot, OwnedSemaphorePermit};

use super::cancel::CancelToken;
pub use super::target::{Operation, UpstreamId};

/// What the ledger records about the upstream a request reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIdentity {
    pub id: UpstreamId,
    /// Stable digest of what the upstream *is* (account identity, or
    /// provider + base URL + model map). Grants are accepted against it.
    pub fingerprint: String,
    /// Bumped whenever the fingerprint changes; a grant names the
    /// generation the device accepted.
    pub generation: u64,
    /// Decides who prices the request: only an Anthropic upstream is
    /// priced by `PriceBook`, which is keyed by model name alone.
    pub kind: super::identity::TargetKind,
}

/// What the handler accepts. Anything else is refused before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub operations: BTreeSet<Operation>,
}

impl Capabilities {
    pub fn accepts(&self, op: Operation) -> bool {
        self.operations.contains(&op)
    }
}

/// A request body read in full, holding its share of the aggregate
/// body budget until it is dropped.
pub struct BufferedBody {
    pub bytes: Bytes,
    pub(crate) _budget: Option<OwnedSemaphorePermit>,
}

impl BufferedBody {
    /// A body with no budget attached — tests and in-process callers.
    pub fn unbudgeted(bytes: Bytes) -> Self {
        Self {
            bytes,
            _budget: None,
        }
    }
}

/// A request that has passed auth, admission, parsing and the grant
/// check. `headers` are the client's, unfiltered: the handler applies its
/// own allowlist, because what may cross differs by upstream kind.
pub struct AdmittedRequest {
    pub operation: Operation,
    pub device_id: String,
    pub beta: bool,
    pub headers: HeaderMap,
    pub body: BufferedBody,
}

pub type ByteStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

pub struct UpstreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: ByteStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// The upstream finished the response.
    Complete,
    /// The stream ended early: client disconnect, cancel, or idle timeout.
    Partial,
    /// No usable response: an error status or a transport failure.
    Failed,
}

/// Token counts in Anthropic's shape, whatever the backend reported.
/// A translated handler normalises into this while it translates (the
/// counts arrive in its own `message_delta`), so the ledger prices one
/// shape and never re-parses a provider's stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

/// A translated provider's own token counters, in its own semantics:
/// `input_tokens` includes the cached ones, and `output_tokens` includes
/// reasoning. Read from the provider's terminal `usage` object while the
/// handler translates, so the ledger still never parses a provider's
/// stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    /// `None` when the provider reported no reasoning count.
    pub reasoning_tokens: Option<u64>,
}

/// The typed result the ledger consumes, so it never re-parses a
/// backend's wire format. The forward handler fills `usage`, `model` and
/// `error` from [`super::sse::UsageTap`] as the body streams past.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub kind: OutcomeKind,
    pub status: Option<u16>,
    /// Time from dispatch to the first response byte.
    pub ttfb: Option<Duration>,
    /// The upstream's `request-id` header — an identifier, not a secret.
    pub upstream_request_id: Option<String>,
    pub bytes: u64,
    pub usage: Option<Usage>,
    /// The provider's own counters. Set only by a translated handler; a
    /// forwarded body is the provider's already, so `usage` is native.
    pub native_usage: Option<ProviderUsage>,
    /// The model that answered, which a mapped route makes different
    /// from the one the client asked for.
    pub model: Option<String>,
    /// The upstream's error type (`overloaded_error`), from an error body
    /// or an in-stream `error` event. An identifier, never upstream prose.
    pub error: Option<String>,
}

/// Refusals caused by the request's *content*, which [`Capabilities`]
/// cannot express because it only sees the operation. Decided in P-D:
/// without this a handler had only `Transport` to return, which renders
/// as a 502 and tells CC the backend is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRefusal {
    /// A tool result references provider state (a Gemini thought
    /// signature) this server does not hold: another device's call, a
    /// session resumed on another server, or a relay restart. The turn is
    /// refused rather than sent without it.
    ContinuationLost,
    /// A translated backend has no token counter. Not 501: CC answers a
    /// gateway 501 with a billable `max_tokens: 1` call (P-D findings).
    CountTokensUnsupported,
    /// Content the backend's capability contract refuses before dispatch.
    ContentUnsupported,
}

pub type OutcomeFuture = oneshot::Receiver<Outcome>;

/// Every way an upstream can fail. A fixed enum: no text from the
/// upstream, from reqwest or from the credential store crosses into a
/// response, a log line or an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamError {
    #[error("credential_unavailable")]
    CredentialUnavailable,
    /// The account became Claude Code's active login on this machine.
    #[error("owner_conflict")]
    OwnerConflict,
    /// The account identity behind the credential changed.
    #[error("identity_changed")]
    IdentityChanged,
    #[error("upstream_unreachable")]
    Transport,
    #[error("upstream_timeout")]
    Timeout,
    #[error("upstream_redirect")]
    Redirect,
    /// Rejected twice: once with the leased token, once after renewal.
    #[error("upstream_credential_rejected")]
    CredentialRejected,
    /// The request itself cannot be served by this upstream.
    #[error("request_refused")]
    Refused(RequestRefusal),
}

#[async_trait::async_trait]
pub trait UpstreamHandler: Send + Sync {
    fn target(&self) -> &TargetIdentity;
    fn capabilities(&self) -> &Capabilities;
    /// Dispatch one admitted request. Returns once response headers are
    /// in hand; the body streams afterwards and resolves the
    /// [`OutcomeFuture`] when it ends or is dropped.
    async fn handle(
        &self,
        req: AdmittedRequest,
        cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError>;
    /// What `GET /relay/health` reports for this upstream. Must not
    /// touch the network: health is polled by every client.
    async fn status(&self) -> super::health::UpstreamStatus {
        super::health::UpstreamStatus::default()
    }
}
