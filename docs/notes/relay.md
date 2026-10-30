# The subscription relay

Working notes, written 2026-10-30 when the relay landed. `AGENTS.md`
states the rules in a line each; this file carries what was measured,
which obvious alternative is wrong, and why.

Read it before changing `claudepot-core::relay`, the `claudepot relay`
verbs, `src-tauri/src/relay_server.rs` or Settings → Relay.

## What it is

Claude Code on another machine runs in **gateway mode**:
`ANTHROPIC_BASE_URL` points at this Mac's relay, `ANTHROPIC_AUTH_TOKEN`
carries a relay device token (`cprl_<64 hex>`), and the relay forwards
`/v1/messages` to Anthropic with an OAuth token from an account registered
here. The remote machine never logs in and never holds an Anthropic
credential.

It is **a behaviour over `account`**, a sibling of `rotation`, not a new
noun. A served account is still an account; the relay only lends its
inference. Provider routes, when they are published (P9), stay in the
existing `routes` store — the relay publishes a subset, it does not grow
a second provider store.

## Separation from Remote Control

The two look alike — a LAN listener, TLS, device tokens — and that is
exactly why they must not share a service. A Remote Control token grants
prompt injection, which is code execution, behind an admin password. A
relay token grants inference billed to an account. Merging them would let
a token minted for one purpose be honoured by the other.

So: separate listener, port, device store, token namespace (`cprl_`) and
**trust root** (`relay-ca.pem`, not the remote CA). Shared code is reused
only as functions taking parameters — `remote::bind::check`, the TLS
material vetting, token generation and hashing, constant-time compare.
Tests assert a relay token is refused by `remote::authenticate` and a
remote token by the relay, and that rotating either CA leaves the other
byte-equal.

## Gateway mode, as measured (CC 2.1.312, 2026-10-29)

- **The `/u/<id>` prefix survives** on every request CC makes, with and
  without a trailing slash: messages, `count_tokens`, startup, compact,
  subagents. That is what lets one base URL select one upstream.
- **Refusals are 403.** CC retries 401 and 503 for 150 s or more; a 403
  makes it exit in about 2 s with the message. Every user-actionable
  refusal (`upstream_not_granted`, `acceptance_required`,
  `owner_conflict`, …) is therefore a 403 carrying a fixed enum.
- **`CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1` is required.** Without it, a
  project's `.claude/settings.json` `env` block can redirect
  `ANTHROPIC_BASE_URL` away from the relay — reproduced. With it, the
  redirect is blocked. `relay::launch` sets it, and clears every
  provider selector and credential variable from `cc-env-spec.json`
  rather than from a hand-typed list.

`crates/xtask/cc-upstream-watch.md` has rows for `relay` and
`relay::launch`; re-run the P0 capture when `cc-drift` flags either.

## Never the CC-active account

The relay refuses to serve the account Claude Code is signed into on this
machine, at start and at every renewal, and stops with `owner_conflict`
if a swap or `/login` makes it active later.

This was measured, not assumed (P0b, on a real account): an outside
renewal rotated the refresh token and invalidated CC's still-unexpired
access token; CC's next refresh then failed and **CC emptied its stored
login**. Everyday Claudepot use rarely shows this because background
refresh never touches the active account and the launcher heals it only
in the last minutes of CC's token. A relay renews for hours. The rule is
strict for that reason; relax it only on new evidence from the installed
binary.

The same mechanism is why a login copied to two machines must not be
served from both: it is one refresh lineage and the two will knock each
other out. Duplicate account fingerprints across servers are a warning
on the client; there is no distributed lock, and none is pretended.

## The token lease, not `get_access_token`

`launcher::get_access_token`'s active-account branch skips the
live-session gate *by design* — its comment justifies that as a
user-initiated launch. A long-running relay is not that. It also returns
a bare `String`, with no expiry and no way to say "this was rejected".

`relay::lease` holds `{ access_token, expires_at, generation }`, renews
five minutes early under the existing credential lock with a re-read,
and on an upstream 401 invalidates *that generation*: the first caller
re-reads the store (another process may already have renewed) and
refreshes only if the stored generation is still the rejected one.
32 concurrent 401s produce one refresh — tested.

## Trust and pairing

- **Own CA, minted on first enable.** The leaf renews under it; the CA
  signing key (`relay-ca-key.pem`, 0600) is needed only for that. A
  missing or corrupt CA key fails closed with `relay_ca_key_unavailable`,
  surfaced as a blocking banner. Minting a fresh CA silently would break
  every pairing without saying so, which is worse than refusing.
- **The leaf renews while serving**, not only at start: a relay left
  running past the 30-day margin would otherwise serve an expired leaf.
  Every six hours `runtime::renew_once` asks `tls::renew_serving` and
  swaps the result into the listener's `LeafResolver`, so the next
  handshake gets it and open connections keep theirs. That check never
  mints, refuses a CA that is not the pinned one, and on any error keeps
  the current leaf and logs only the error code. The code is also kept on
  `RelayState` (`LiveStatus::trust_fault`), and the GUI folds it into
  `lastErrorCode`, so `relay_ca_key_unavailable` raises the same blocking
  banner mid-serve as at start — while the old leaf still has time left.
  The next renewal that succeeds clears it.
- **`claudepot relay rotate-ca --yes` replaces the CA**, only while no
  relay serves (`service::rotate_ca`). It revokes every device first:
  clients pinned the old CA, and a token left live beside a new CA could
  fetch it from the public `GET /relay/ca` and carry on, so "rotation
  ends every pairing" has to be true on the server too.
- **Pairing is fingerprint-first.** The client fetches `GET /relay/ca`
  (public material only), the user matches the SHA-256 fingerprint out of
  band — `--fingerprint` is mandatory non-interactively — and only a
  CA-pinned, SAN-checked connection then carries the one-use code. A
  substituted CA at fetch, or a different chain on reconnect, sends no
  secret: both are tested with a MITM that records what it sees.
- **The service always serves TLS**, loopback included, so there is one
  pairing path. The plan's separate loopback enrollment was dropped.

## Binding is enforced per request, on the server

Every published upstream has a *target fingerprint*: a digest of what
decides where a request goes and who pays (kind, origin, model mapping,
credential identity — never the key itself). Each device grant records
the generation, and the config epoch, it accepted. Changing the served
account bumps the generation, and from then on that device's requests
get 403 `acceptance_required` **before any upstream contact**, until
`claudepot relay accept` records the new fingerprint. A CC session
already running against account A reaches account B zero times — tested
both between health check and admission and mid-session.

Grants are a snapshot: an upstream published after pairing is not
reachable by an old token. "Include future publications" is an explicit
per-device choice, and the pane states what it spends.

The epoch exists because a reset `relay-config.json` restarts generation
numbers; without it, an old grant could match a reissued number.

## Secrets in flight

One `Secret` type with redacted `Debug`/`Display`, zeroized on drop;
outbound credential headers are `set_sensitive`. Raw requests,
responses, headers and upstream error bodies are never logged, and the
public error is a fixed enum — `relay-status`'s `last_error` carries the
enum, never upstream text. The canary test drives distinct synthetic
OAuth and device tokens through malformed requests, transport and
upstream failures, TRACE logging and a panic, and asserts neither appears
anywhere. It runs in a child process: tracing's process-wide
callsite-interest cache made it flaky in-suite.

## Streaming

Request bodies are buffered (≤ 32 MiB, 256 MiB aggregate) — that is what
makes a single 401 retry possible, and the retry happens only if no
response byte has been sent. Responses stream unbuffered through
`bytes_stream()`. The admission permit lives inside the response body,
so a stream holds its slot until it ends or the client drops, and a drop
cancels the upstream request. The relay's HTTP client is its own: the
OAuth client's 15 s total timeout would kill a long thinking turn, so
this one has a resetting 10-minute read-idle timeout and no total
deadline. Revoking a device cancels its in-flight streams.

## Running it: one process, explicit start

- **The serve lock** (`relay-serve.lock`) says whether any process —
  the GUI or a terminal `claudepot relay serve` — is serving. While it
  is held, offline edits to `relay-config.json` and `relay-devices.json`
  are refused: a running server holds them in memory and would not see
  the edit. The lock belongs to the relay's state, not its listener, so
  it is released only when nothing that can still write those files is
  left — dropping a server without draining it cannot admit a second
  writer.
- **`relay revoke` from another shell goes through the serving relay**,
  over its revoke socket (`relay::revoke_socket`, `relay-revoke/socket`,
  Unix only), so the device is refused at once and its streams end.
  Revoke is the socket's only verb because it only takes access away: a
  same-user process could already stop the relay, so it grants nothing
  new, while a verb that adds access would. The peer's uid is the
  boundary and both ends check it — the relay closes a connection from
  another uid unread, and the client sends nothing to a socket another
  uid listens on, so it cannot be steered into a forged "revoked". The
  directory is created 0700 and refused, never repaired: a chmod would
  hide that someone else could already have written there. A relay that
  cannot bind it still serves and says why; revoking then means stopping
  it first, as it always does on Windows. A reply that never arrives is
  an unknown outcome, not a failure, and revoking again is safe because
  revoke is idempotent. `dev-docs/relay-revoke-socket.md` has the
  interleavings.
- **Settings → Relay revokes through whichever relay serves.** Its own
  relay revokes in-process; a terminal relay is reached over the same
  socket (`service::revoke_anywhere`), and Revoke is disabled with an
  inline reason only when that relay left no socket — always the case on
  Windows. `relay.revoke_outcome_unknown` is the GUI's name for an
  unanswered request, and its toast says "maybe revoked, safe to
  repeat", never "could not be changed". When the GUI's own relay could
  not bind its socket, the pane says a terminal `relay revoke` cannot
  reach it and why (`liveRevokeUnavailable`); on Windows that is the
  platform, not a fault, and the pane stays quiet.
- **The GUI's relay runs in-process, and quitting stops it.** Start and
  Stop are explicit; there is no "enabled" preference that restarts it
  at boot. A relay started from a terminal shows as serving, and the
  pane locks its controls rather than offering a Stop it cannot honour.
- **Signals**: the status-bar chip is the ambient tier; owner conflict,
  identity change and an unreadable CA key are blocking banners in the
  existing status-issue stack; other upstream errors are one durable
  `relayError` log row each. A blocking code never also reaches the log.

## Usage and cost

`relay-usage.jsonl` is append-only, holds no content and no secret, and
records per request the device, upstream and target fingerprint, the
requested **and** the answering model, the outcome
(`complete | partial | failed`, with an in-stream error after HTTP 200
counted as failed), TTFB and usage. Cost uses `PriceBook::resolve` only
for Anthropic upstreams and is shown with `≈`; a client asking for
`sonnet` that a route maps to another provider would otherwise be priced
as Claude, so every non-Anthropic upstream renders **unpriced**.

Usage is a pair. `normalized` is what the client saw, in Anthropic's
shape. `native` is the provider's own counters: for a forwarded upstream
the two are the same bytes; for a translated route the handler copies
the provider's terminal `usage` out while translating, in the provider's
semantics (input *includes* cached, reasoning sits inside output). A
provider that sent no usage records `native: null`, never the zeros the
Chat hop synthesizes to stay byte-identical with the reference port.

## Choosing a server

The user picks the upstream; scoring only picks *which server* serves
it. Eligibility first (fingerprint approved, generation accepted,
credential ready, capacity, no 429 cooldown, utilisation below 95 % of
5 h / 98 % of 7 d when known). Then a server with fresh TTFB samples for
that model always outranks one only measured by connect time: adding
connect time and TTFB into one number would let a cold server win
because its inference cost was never observed. A utilisation whose reset
time has passed counts as unknown, not as high.

## When a client can connect but never pairs

Seen on the first real-tailnet run (2026-10-30): TCP to the relay port
connected, the TLS handshake then hung until timeout, and the server
never accepted the socket — while the same binary served loopback
fine and a Python listener on the same tailnet address answered. The
cause was the macOS Application Firewall in **stealth mode**: a binary
not on its allow list has inbound connections dropped silently, with
nothing in the relay's logs, because the relay never sees them. A
freshly built or re-signed `claudepot` is a new binary to the firewall.
Check `socketfilterfw --getstealthmode --listapps` before debugging the
relay; the fix is allowing the binary, which is a system setting the
relay cannot change for you.

## Provider drift

Translated providers (P10) drift on the provider's side, not CC's, so
they have their own list, `crates/xtask/provider-upstream-watch.md`,
beside `cc-upstream-watch.md` — not a column in the CC list, which
`cargo xtask cc-drift` reads as CC surfaces only.

## Not yet built

- `RouteTarget::Relay` (P4-R) was decided against: relay servers stay
  out of `routes.json`. `relay-servers.json` is the client's only
  record, the Providers tab lists it read-only beside the routes
  (`relay_client_servers`, which sends fixed codes, never the probe's
  or the health report's text), and `claudepot relay run` is the launch
  path. Pairing, acceptance and launching from the GUI are not built.
- Live legs of translated upstreams: no Bash tool round trip has run
  against Codex or OpenAI yet (no relay Codex login, no vendor key on the
  build host), so clai stays in use for every provider.
- Publishing a route from the GUI: `claudepot relay publish-route` is
  the only way in today.

## Pass-through upstreams (plan P9)

`claudepot relay publish-route <route>` publishes a **gateway** route
(Providers tab) as an upstream; Bedrock, Vertex and Foundry routes are
refused as `NotRelayCapable`, because they need the provider's own auth
on the client, not a key. The publication records `route_id` instead of
`account_id` in `relay-config.json` — one or the other, never both — and
the route itself stays in `routes.json`; the relay never copies it.

`relay::passthrough` forwards Anthropic bytes with two changes only:

- **`model` is rewritten from the route** (`map_model`): a route model
  is kept, a haiku-tier request goes to the small/fast model, anything
  else to the primary model. The body is re-encoded, so top-level key
  order is not preserved. A body that is not a JSON object is
  `content_unsupported` before dispatch, never forwarded unmapped.
- **The credential is the route's key**, read per request (a rotation
  needs no restart) and always sent as `Authorization: Bearer` — CC's
  CLI sends `ANTHROPIC_AUTH_TOKEN` that way whatever the route's Desktop
  auth scheme says. The client's allowlisted headers pass; the OAuth
  beta is never added, and the device token never leaves.

**The target is the route's kind, origin, model mapping and the key's
*location*, never the key.** Rotating a key in place keeps the
generation; editing the origin or a model, or moving the key between
plaintext and the keychain, bumps it at the next start, and every device
gets `acceptance_required` before a byte reaches the new target. The
origin is fixed when the handler is built, so an edit reaches a serving
relay only through a restart. A 401 from the gateway is
`upstream_credential_rejected` with no retry — a route key has no
renewal. Route upstreams are never priced.

## Route secrets (plan §5.5)

`relay::route_secret` reads a published route's key: `routes.json` for a
plaintext route, the keychain cell `routes::keychain` writes for a
keychain route. A locked or empty keychain is an error the relay renders
as `credential_unavailable`, never an empty key.

The plan called for migrating keychain route secrets off per-binary
`keyring` items. There was nothing to migrate: the workspace enables no
`keyring` backend, so `keyring` was its in-memory mock and no route
secret ever reached the keychain. On macOS `routes::keychain` now writes
through `/usr/bin/security` (`security_cli`), which the wrapper's helper,
the GUI and a headless relay all read. A route saved with "store in
keychain" before that has no key anywhere and must be re-entered. Linux
and Windows still sit on the mock until a backend feature is chosen.

## Translated upstreams (plan P10)

`claudepot relay publish-route <route> --translate <protocol>` publishes
a gateway route behind `relay::translate`: Anthropic Messages in, the
provider's protocol out, and back. The protocol is one of `codex`,
`openai`, `grok` (Responses), `gemini` or `chat` (Chat Completions). It is
recorded on the publication (`translate` in `relay-config.json`), needs a
`route_id`, and is part of the target: switching it, or switching it
off, is a new generation and `acceptance_required` on every device.

- **Ported, not rewritten.** Every hop is clai's `clcd` filter, replayed
  against the 40 captured cases in `testdata/translate/`
  (`translate::fixtures`, exact tallies). Where Claudepot deliberately
  differs — model from the route's map, upstream error text masked, an
  unfinished tool call is an error — the step counts as a deviation, not
  a match. Swapping two call ids in a round trip breaks the replay; a
  test holds that.
- **What is refused before dispatch, as 400**: `count_tokens`
  (`count_tokens_unsupported` — never an estimate, and never 501, which
  CC answers with a billable call); content the protocol cannot carry
  (`content_unsupported`); a Gemini tool result whose thought signature
  this process does not hold (`continuation_state_lost`).
- **Continuation state is memory-only**, one store per serve, keyed by
  device, target, model and call id. Another device, another server or a
  restart loses it, and the turn is refused rather than sent without the
  signature. CC shows the error; a new turn recovers.
- **Upstream text never reaches the client.** A provider error is
  re-stated with fixed text, its status and `Retry-After` kept; a stream
  fault ends the stream with a fixed `error` event.
- **Every buffer is capped** (`translate::MAX_*`). A provider that streams
  forever in tiny fragments costs that one request a bounded amount and
  is dropped; the subscription beside it keeps serving. A client
  disconnect or a revocation drops the provider connection.
- **Codex pays with the relay's own Codex login**, never the user's:
  sign it in once with `mkdir -p -m 700 ~/.claudepot/relay-codex &&
  CODEX_HOME=~/.claudepot/relay-codex codex login`. The `mkdir` is not
  optional: codex refuses a `CODEX_HOME` that does not exist yet.
  The relay asks Codex's own `app-server` for the token and never runs the
  exchange itself, because two owners of one refresh token double-spent
  it in every overlapping trial (P-D). Until the home is signed in,
  publishing a Codex route is refused and serving one is
  `credential_unavailable`. The `codex` binary is found on `PATH`, which
  a GUI started by launchd may not have.
- Translated upstreams are never priced.
