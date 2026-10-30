//! [`TranslateHandler`]: the `Translated` upstream behind the relay's
//! shared admission, target and ledger path.
//!
//! The provider is always asked to stream; a client that did not ask for
//! a stream gets the collected message as one JSON body. Cancellation is
//! the server's: it stops polling the body when a device is revoked, and
//! dropping the body drops the provider connection with it. Every buffer
//! on the way is capped (`super::MAX_*`), so a provider that streams
//! forever in tiny fragments costs that request a bounded amount of
//! memory and nothing else.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use futures_util::StreamExt;
use serde_json::Value;

use super::super::cancel::CancelToken;
use super::super::forward::{
    bearer, execute, relay_client, report_stream, Cooldown, NativeSlot, Reported, Timeouts,
};
use super::super::health::UpstreamStatus;
use super::super::passthrough::{
    credential_digest, key_digest, map_model, CredentialHealth, ReadTicket,
};
use super::super::route_secret::{KeychainReader, RouteKey};
use super::super::secret::Secret;
use super::super::sse::Mode;
use super::super::upstream::{
    AdmittedRequest, Capabilities, Operation, OutcomeFuture, ProviderUsage, RequestRefusal,
    TargetIdentity, UpstreamError, UpstreamHandler, UpstreamResponse,
};
use super::chat::{to_chat, ChatError, ChatToResponses, Out};
use super::codex::CodexAuth;
use super::continuation::{ContinuationStore, Key};
use super::errors::{anthropic_error, MAX_ERROR_BODY};
use super::request::{to_responses, Names, RequestOpts, DEFAULT_EFFORT};
use super::stream::{encode, Collector, Event, Fault, Line, ResponsesToAnthropic, SseLines};
use super::{endpoint, Translation};
use crate::routes::{Route, RouteProvider};

/// Who pays for a translated route.
pub enum Credential {
    /// The route's key, read from the given route through `keychain` on
    /// every request.
    RouteKey(Arc<dyn KeychainReader + Send + Sync>),
    /// The route's key from a [`RouteKey`], which can also re-read
    /// `routes.json` — what the production handler uses.
    Route(RouteKey),
    /// The relay's own Codex login.
    Codex(Arc<CodexAuth>),
}

/// Where a request's credential comes from, once the handler is built.
#[derive(Clone)]
enum Source {
    Key(RouteKey),
    Codex(Arc<CodexAuth>),
}

impl Source {
    async fn read(&self, route: crate::routes::RouteId) -> Result<Auth, UpstreamError> {
        match self {
            Source::Key(key) => key.read().await.map(Auth::Key).map_err(|e| {
                tracing::warn!(route = %route, error = %e, "relay route secret unavailable");
                UpstreamError::CredentialUnavailable
            }),
            Source::Codex(auth) => auth.token().await.map(Auth::Codex).map_err(|e| {
                tracing::warn!(route = %route, error = %e, "relay codex credential unavailable");
                UpstreamError::CredentialUnavailable
            }),
        }
    }

    /// The background re-read health runs while a failure stands. Codex
    /// goes back to the app-server rather than the cache, so a home signed
    /// in again after a rejection is seen without a request.
    async fn probe(
        self,
        route: crate::routes::RouteId,
    ) -> super::super::passthrough::CredentialRead {
        match self {
            Source::Codex(auth) => auth
                .reread()
                .await
                .map(|t| t.digest())
                .map_err(|_| UpstreamError::CredentialUnavailable),
            key => key.read(route).await.map(|a| a.digest()),
        }
    }
}

pub struct TranslateHandler {
    target: TargetIdentity,
    caps: Capabilities,
    route: Route,
    translation: Translation,
    url: String,
    client: reqwest::Client,
    timeouts: Timeouts,
    source: Source,
    continuations: Arc<ContinuationStore>,
    cooldown: Cooldown,
    health: Arc<CredentialHealth>,
}

fn refuse(r: RequestRefusal) -> UpstreamError {
    UpstreamError::Refused(r)
}

impl TranslateHandler {
    /// The production handler: the route's current key (`routes.json` or
    /// the OS keychain) for a keyed route, and the relay's Codex home —
    /// one [`CodexAuth`] per home, however many publications share it —
    /// for a Codex route.
    ///
    /// `routes_file` is the serving data dir's `routes.json`, re-read on
    /// every request for a keyed route.
    pub fn new(
        target: TargetIdentity,
        route: Route,
        translation: Translation,
        continuations: Arc<ContinuationStore>,
        routes_file: std::path::PathBuf,
    ) -> reqwest::Result<Self> {
        let timeouts = Timeouts::default();
        let credential = match translation {
            Translation::Codex => {
                Credential::Codex(CodexAuth::shared("codex", super::codex::default_home()))
            }
            _ => Credential::Route(RouteKey::production(route.clone(), routes_file)),
        };
        Ok(Self::with_client(
            target,
            route,
            translation,
            relay_client(&timeouts)?,
            timeouts,
            credential,
            continuations,
        ))
    }

    /// Everything injectable, for tests against a loopback provider.
    pub fn with_client(
        target: TargetIdentity,
        route: Route,
        translation: Translation,
        client: reqwest::Client,
        timeouts: Timeouts,
        credential: Credential,
        continuations: Arc<ContinuationStore>,
    ) -> Self {
        let origin = match &route.provider {
            RouteProvider::Gateway(cfg) => cfg.base_url.trim().trim_end_matches('/').to_owned(),
            _ => String::new(),
        };
        let source = match credential {
            Credential::RouteKey(keychain) => Source::Key(RouteKey::new(route.clone(), keychain)),
            Credential::Route(key) => Source::Key(key),
            Credential::Codex(auth) => Source::Codex(auth),
        };
        Self {
            target,
            caps: Capabilities {
                // `count_tokens` is admitted so it is refused with its
                // own code, not as an unknown operation.
                operations: [Operation::Messages, Operation::CountTokens].into(),
            },
            url: endpoint(&origin, translation),
            route,
            translation,
            client,
            timeouts,
            source,
            continuations,
            cooldown: Cooldown::default(),
            health: Arc::default(),
        }
    }

    /// Read the credential once, as a start check.
    pub async fn check_credential(&self) -> Result<(), UpstreamError> {
        self.credential().await.map(|_| ())
    }

    /// The credential, and the ticket its read took.
    async fn credential(&self) -> Result<(Auth, ReadTicket), UpstreamError> {
        let ticket = self.health.begin();
        let r = self.source.read(self.route.id).await;
        self.health.read(
            ticket,
            r.as_ref().map(Auth::digest).map_err(|e| *e),
            Instant::now(),
        );
        r.map(|a| (a, ticket))
    }

    fn build(&self, body: &[u8], auth: &Auth) -> Result<reqwest::Request, UpstreamError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        match auth {
            Auth::Key(Some(k)) => {
                headers.insert(header::AUTHORIZATION, bearer(k.expose())?);
            }
            Auth::Key(None) => {}
            Auth::Codex(t) => {
                headers.insert(header::AUTHORIZATION, bearer(t.token.expose())?);
                headers.insert(
                    "chatgpt-account-id",
                    HeaderValue::from_str(&t.account_id)
                        .map_err(|_| UpstreamError::CredentialUnavailable)?,
                );
                headers.insert(
                    "openai-beta",
                    HeaderValue::from_static("responses=experimental"),
                );
                headers.insert("originator", HeaderValue::from_static("codex_cli_rs"));
            }
        }
        self.client
            .post(&self.url)
            .headers(headers)
            .body(body.to_vec())
            .build()
            .map_err(|_| UpstreamError::Transport)
    }

    /// The shared send: a header timeout, and a redirect refused as
    /// `upstream_redirect` — the provider's origin is never followed.
    async fn send(&self, body: &[u8], auth: &Auth) -> Result<reqwest::Response, UpstreamError> {
        let request = self.build(body, auth)?;
        execute(&self.client, request, self.timeouts.response_header).await
    }

    /// The provider request, or the refusal that stops it before dispatch.
    fn translate(&self, req: &AdmittedRequest) -> Result<Prepared, UpstreamError> {
        let content = refuse(RequestRefusal::ContentUnsupported);
        let v: Value = serde_json::from_slice(&req.body.bytes).map_err(|_| content)?;
        if !v.is_object() {
            return Err(content);
        }
        let client_model = v.get("model").and_then(Value::as_str);
        let model = map_model(&self.route, client_model).to_owned();
        let names = Names::for_request(&v);
        let opts = RequestOpts {
            model: model.clone(),
            effort: DEFAULT_EFFORT.to_owned(),
            effort_override: false,
            translation: self.translation,
        };
        let responses = to_responses(&v, &names, &opts).map_err(|e| {
            tracing::debug!(route = %self.route.id, reason = %e, "relay translate refused a request");
            content
        })?;
        let upstream = if self.translation.is_chat() {
            let gemini = self.translation == Translation::Gemini;
            let lookup = |call_id: &str| self.continuations.get(&self.key(req, &model, call_id));
            match to_chat(&responses, gemini, lookup) {
                Ok(c) => c,
                Err(ChatError::MissingMetadata) => {
                    return Err(refuse(RequestRefusal::ContinuationLost))
                }
                Err(ChatError::Unsupported(why)) => {
                    tracing::debug!(route = %self.route.id, reason = why, "relay translate refused a request");
                    return Err(content);
                }
            }
        } else {
            responses
        };
        let body = serde_json::to_vec(&upstream).map_err(|_| content)?;
        Ok(Prepared {
            body,
            model,
            names,
            stream: v.get("stream").and_then(Value::as_bool) == Some(true),
        })
    }

    /// The provider rejected `auth`. A Codex token can be renewed once; a
    /// route key cannot. Either way a rejection that stands is recorded,
    /// so health reports it until [`CredentialHealth`] clears it.
    /// `ticket` is the read that returned `auth`; a renewal counts as part
    /// of that read.
    async fn renewed(&self, auth: &Auth, ticket: ReadTicket) -> Result<Auth, UpstreamError> {
        let (Auth::Codex(t), Source::Codex(codex)) = (auth, &self.source) else {
            return Err(self.rejected(auth, ticket));
        };
        match codex.after_rejection(&t.token).await {
            Ok(renewed) => Ok(Auth::Codex(renewed)),
            Err(_) => Err(self.rejected(auth, ticket)),
        }
    }

    fn rejected(&self, auth: &Auth, ticket: ReadTicket) -> UpstreamError {
        self.health.rejected(ticket, auth.digest(), Instant::now());
        UpstreamError::CredentialRejected
    }

    fn key(&self, req: &AdmittedRequest, model: &str, call_id: &str) -> Key {
        Key {
            device_id: req.device_id.clone(),
            fingerprint: self.target.fingerprint.clone(),
            model: model.to_owned(),
            call_id: call_id.to_owned(),
        }
    }
}

enum Auth {
    Key(Option<Secret>),
    Codex(super::codex::CodexToken),
}

impl Auth {
    fn digest(&self) -> [u8; 32] {
        match self {
            Auth::Key(k) => key_digest(k),
            Auth::Codex(t) => t.digest(),
        }
    }
}

impl super::codex::CodexToken {
    /// What [`CredentialHealth`] tracks for a Codex token.
    fn digest(&self) -> [u8; 32] {
        credential_digest(self.token.expose())
    }
}

struct Prepared {
    body: Vec<u8>,
    model: String,
    names: Names,
    stream: bool,
}

/// One chat-hop output: a Responses event goes on to the caller, a tool
/// call's replay metadata is kept for the next turn.
fn dispatch(o: Out, store: &ContinuationStore, key: &Key) -> Option<Value> {
    match o {
        Out::Event(e) => Some(e),
        Out::Cache {
            id,
            model,
            extra_content,
        } => {
            store.put(
                Key {
                    call_id: id,
                    model,
                    ..key.clone()
                },
                extra_content,
            );
            None
        }
    }
}

/// The provider's own counters, or `None` while a chat hop has not yet
/// seen the provider report usage — Responses-shaped usage the chat hop
/// synthesised is not the provider's. Shared by both paths.
fn native_usage(
    chat: Option<&ChatToResponses>,
    usage: impl FnOnce() -> Option<ProviderUsage>,
) -> Option<ProviderUsage> {
    if chat.is_some_and(|c| !c.usage_reported()) {
        return None;
    }
    usage()
}

/// Provider bytes in, Responses events out: SSE lines, then the chat hop
/// when there is one, with each call's replay metadata kept. The one
/// provider-side pipeline — the streaming [`Pipeline`] and the collected
/// [`collect`] differ only in where the events go.
struct Hops {
    lines: SseLines,
    chat: Option<ChatToResponses>,
    store: Arc<ContinuationStore>,
    key: Key,
}

/// The events one step produced, in order, and the fault that stopped it.
type Step = (Vec<Value>, Option<Fault>);

impl Hops {
    fn line(&mut self, line: &Line, out: &mut Vec<Value>) -> Result<(), Fault> {
        match self.chat.as_mut() {
            None => {
                if let Line::Data(d) = line {
                    out.push(super::stream::parse_event(d)?);
                }
            }
            Some(chat) => {
                for o in chat.feed_line(line)? {
                    out.extend(dispatch(o, &self.store, &self.key));
                }
            }
        }
        Ok(())
    }

    fn lines(&mut self, lines: &[Line]) -> Step {
        let mut out = Vec::new();
        for l in lines {
            if let Err(f) = self.line(l, &mut out) {
                return (out, Some(f));
            }
        }
        (out, None)
    }

    fn feed(&mut self, chunk: &[u8]) -> Step {
        // The events a chunk completed before a framing fault still go
        // out ahead of it, in the same step.
        let (lines, framing) = self.lines.feed_until_fault(chunk);
        let (out, fault) = self.lines(&lines);
        (out, fault.or(framing))
    }

    /// The provider's body ended.
    fn finish(&mut self) -> Step {
        let (mut out, fault) = match self.lines.finish() {
            Ok(last) => self.lines(last.as_slice()),
            Err(f) => (Vec::new(), Some(f)),
        };
        if fault.is_some() {
            return (out, fault);
        }
        if let Some(chat) = self.chat.as_mut() {
            match chat.finish() {
                Ok(outs) => {
                    for o in outs {
                        out.extend(dispatch(o, &self.store, &self.key));
                    }
                }
                Err(f) => return (out, Some(f)),
            }
        }
        (out, None)
    }

    fn native(&self, usage: impl FnOnce() -> Option<ProviderUsage>) -> Option<ProviderUsage> {
        native_usage(self.chat.as_ref(), usage)
    }
}

/// [`Hops`] into Anthropic SSE, for one streaming request.
struct Pipeline {
    hops: Hops,
    anthropic: ResponsesToAnthropic,
    native: NativeSlot,
}

impl Pipeline {
    /// Copy the provider's counters out for the ledger once known.
    fn note_native(&self) {
        if let Some(n) = self.hops.native(|| self.anthropic.native_usage()) {
            *self.native.lock() = Some(n);
        }
    }

    fn deliver(&mut self, (events, fault): Step) -> Vec<Event> {
        let mut out = Vec::new();
        for e in &events {
            out.extend(self.anthropic.feed_event(e));
        }
        if let Some(f) = fault {
            out.extend(self.anthropic.fail(f));
        }
        out
    }

    fn feed(&mut self, chunk: &[u8]) -> Bytes {
        if self.anthropic.ended() {
            return Bytes::new();
        }
        let step = self.hops.feed(chunk);
        let events = self.deliver(step);
        self.note_native();
        encode(&events)
    }

    fn end(&mut self, fault: Option<Fault>) -> Bytes {
        let mut events = Vec::new();
        if let Some(f) = fault {
            events.extend(self.anthropic.fail(f));
        }
        let step = self.hops.finish();
        events.extend(self.deliver(step));
        events.extend(self.anthropic.finish());
        self.note_native();
        encode(&events)
    }
}

/// What [`collect`] returns: the message, the provider's counters, and
/// when the provider's first body byte arrived.
type Collected = (
    Result<(Value, Option<ProviderUsage>), Fault>,
    Option<Instant>,
);

/// The non-streaming path: the same [`Hops`] into a [`Collector`]. The
/// first byte's arrival is kept even on a fault, because that is the
/// latency the provider showed, and the collected body is only sent after
/// it all arrived.
async fn collect(resp: reqwest::Response, hops: Hops, model: &str, names: &Names) -> Collected {
    let mut first_byte = None;
    let r = collect_into(resp, hops, model, names, &mut first_byte).await;
    (r, first_byte)
}

/// Read until the first terminal event or fault — not until EOF, which a
/// provider sending heartbeats after its answer may never send — then
/// drop the provider connection.
async fn collect_into(
    resp: reqwest::Response,
    mut hops: Hops,
    model: &str,
    names: &Names,
    first_byte: &mut Option<Instant>,
) -> Result<(Value, Option<ProviderUsage>), Fault> {
    fn absorb(collector: &mut Collector, (events, fault): Step) {
        for e in events {
            collector.feed_event(e);
        }
        if let Some(f) = fault {
            collector.fail(f);
        }
    }
    let mut collector = Collector::default();
    let mut body = resp.bytes_stream();
    while !collector.done() {
        match body.next().await {
            None => {
                absorb(&mut collector, hops.finish());
                break;
            }
            Some(Err(_)) => collector.fail(Fault::Truncated),
            Some(Ok(chunk)) => {
                if first_byte.is_none() && !chunk.is_empty() {
                    *first_byte = Some(Instant::now());
                }
                absorb(&mut collector, hops.feed(&chunk));
            }
        }
    }
    drop(body);
    let native = hops.native(|| collector.native_usage());
    collector.finish(model, names).map(|m| (m, native))
}

/// A body the handler writes whole, after the provider answered.
/// `ttfb` is the provider's latency to its first byte, measured while
/// the body was read; the outcome reports it rather than the near-zero
/// time from here to the first poll of an already-collected body.
fn json_response(
    status: StatusCode,
    body: &Value,
    extra: HeaderMap,
    native: Option<ProviderUsage>,
    ttfb: Duration,
) -> (UpstreamResponse, OutcomeFuture) {
    let mut headers = extra;
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let bytes = Bytes::from(body.to_string());
    report_stream(
        futures_util::stream::iter([Ok::<_, std::io::Error>(bytes)]),
        Reported {
            status,
            headers,
            request_id: None,
            mode: Mode::Json,
            // `report_stream` measures from `started` to the first chunk,
            // which is polled at once; back-dating by `ttfb` makes it
            // report the provider's latency.
            started: {
                let now = Instant::now();
                now.checked_sub(ttfb).unwrap_or(now)
            },
            native: native.map(|n| Arc::new(parking_lot::Mutex::new(Some(n)))),
        },
    )
}

async fn bounded_body(resp: reqwest::Response) -> Vec<u8> {
    let mut out = Vec::new();
    let mut body = resp.bytes_stream();
    while let Some(Ok(chunk)) = body.next().await {
        let room = MAX_ERROR_BODY.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if out.len() >= MAX_ERROR_BODY {
            break;
        }
    }
    out
}

#[async_trait::async_trait]
impl UpstreamHandler for TranslateHandler {
    fn target(&self) -> &TargetIdentity {
        &self.target
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    async fn status(&self) -> UpstreamStatus {
        self.health.status(
            self.cooldown.remaining(),
            self.source.clone().probe(self.route.id),
        )
    }

    async fn handle(
        &self,
        req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        if req.operation == Operation::CountTokens {
            return Err(refuse(RequestRefusal::CountTokensUnsupported));
        }
        let prepared = self.translate(&req)?;
        // A redirect never gets here: `send` refuses it.
        let (resp, started) = self.dispatch_authenticated(&prepared.body).await?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            self.cooldown.start(resp.headers());
        }
        if !status.is_success() {
            return Ok(error_response(resp, started).await);
        }
        let hops = Hops {
            lines: SseLines::default(),
            chat: self
                .translation
                .is_chat()
                .then(|| ChatToResponses::new(&prepared.model, !prepared.stream)),
            store: self.continuations.clone(),
            key: self.key(&req, &prepared.model, ""),
        };
        if prepared.stream {
            Ok(streaming_response(resp, hops, prepared, started))
        } else {
            Ok(collected_response(resp, hops, &prepared, started).await)
        }
    }
}

impl TranslateHandler {
    /// Read the credential, send, and on a 401 renew once and resend the
    /// same body. The provider's answer, and when the first send began.
    ///
    /// Acceptance clears only a rejection of the credential this request
    /// used: an older request answered late must not clear a newer
    /// rejection of a different key.
    async fn dispatch_authenticated(
        &self,
        body: &[u8],
    ) -> Result<(reqwest::Response, Instant), UpstreamError> {
        let (mut auth, ticket) = self.credential().await?;
        let started = Instant::now();
        let mut resp = self.send(body, &auth).await?;
        if resp.status() == StatusCode::UNAUTHORIZED {
            auth = self.renewed(&auth, ticket).await?;
            resp = self.send(body, &auth).await?;
            if resp.status() == StatusCode::UNAUTHORIZED {
                return Err(self.rejected(&auth, ticket));
            }
        }
        self.health.accepted_credential(auth.digest());
        Ok((resp, started))
    }
}

/// A provider error, re-stated with fixed text; `Retry-After` is kept.
async fn error_response(
    resp: reqwest::Response,
    started: Instant,
) -> (UpstreamResponse, OutcomeFuture) {
    let status = resp.status();
    let mut extra = HeaderMap::new();
    if let Some(v) = resp.headers().get(header::RETRY_AFTER) {
        extra.insert(header::RETRY_AFTER, v.clone());
    }
    let ttfb = started.elapsed();
    let body = bounded_body(resp).await;
    json_response(status, &anthropic_error(status, &body), extra, None, ttfb)
}

/// The client did not ask for a stream: collect, then answer once.
async fn collected_response(
    resp: reqwest::Response,
    hops: Hops,
    prepared: &Prepared,
    started: Instant,
) -> (UpstreamResponse, OutcomeFuture) {
    let headers_at = Instant::now();
    let (collected, first_byte) = collect(resp, hops, &prepared.model, &prepared.names).await;
    let ttfb = first_byte
        .unwrap_or(headers_at)
        .saturating_duration_since(started);
    match collected {
        Ok((message, native)) => {
            json_response(StatusCode::OK, &message, HeaderMap::new(), native, ttfb)
        }
        Err(f) => {
            let body = serde_json::json!({
                "type": "error",
                "error": {"type": "api_error", "message": f.message()},
            });
            json_response(StatusCode::BAD_GATEWAY, &body, HeaderMap::new(), None, ttfb)
        }
    }
}

/// The client asked for a stream: translate chunk by chunk, and stop
/// reading the provider at the terminal event or a fault.
fn streaming_response(
    resp: reqwest::Response,
    hops: Hops,
    prepared: Prepared,
    started: Instant,
) -> (UpstreamResponse, OutcomeFuture) {
    let native: NativeSlot = Arc::default();
    let pipeline = Pipeline {
        hops,
        anthropic: ResponsesToAnthropic::new(&prepared.model, prepared.names),
        native: native.clone(),
    };
    let upstream = resp.bytes_stream().boxed();
    let body = futures_util::stream::unfold(Some((upstream, pipeline)), |state| async move {
        let (mut upstream, mut p) = state?;
        loop {
            match upstream.next().await {
                Some(Ok(chunk)) => {
                    let out = p.feed(&chunk);
                    if p.anthropic.ended() {
                        // Terminal event or fault: stop reading the
                        // provider and close the client stream.
                        let tail = p.end(None);
                        return Some((Ok::<_, std::io::Error>(join(out, tail)), None));
                    }
                    if !out.is_empty() {
                        return Some((Ok(out), Some((upstream, p))));
                    }
                }
                Some(Err(_)) => return Some((Ok(p.end(Some(Fault::Truncated))), None)),
                None => return Some((Ok(p.end(None)), None)),
            }
        }
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    report_stream(
        body,
        Reported {
            status: StatusCode::OK,
            headers,
            request_id: None,
            mode: Mode::Sse,
            started,
            native: Some(native),
        },
    )
}

fn join(a: Bytes, b: Bytes) -> Bytes {
    if b.is_empty() {
        return a;
    }
    let mut v = a.to_vec();
    v.extend_from_slice(&b);
    Bytes::from(v)
}

#[cfg(test)]
#[path = "tests_handler.rs"]
mod tests_handler;
