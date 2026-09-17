# Remote control and the phone panel

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries the appliance security model, the certificate work, and every design decision inside the panel client.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## Remote control (LAN appliance, admin password)

Reaching Claudepot from a phone or another machine. Pure logic in
`claudepot-core::remote` (`bind` / `password` / `token` / `tls` /
`store` + the pairing state machine in `mod`).

**The model is an appliance**, like Home Assistant or a NAS: reachable
on the LAN — over Tailscale or not — behind one admin password, and
whoever holds that password is admin and may do anything. There is no
endpoint allowlist. That is coherent *only* because the password is
treated as the entire security boundary, which is what the rest of this
section is about: behind it sits the ability to drive Claude Code
sessions, i.e. arbitrary code execution as this user.

**`bind` is an allowlist and the highest-consequence line in the
feature.** Permitted: loopback, RFC1918, link-local, Tailscale's
`100.64.0.0/10`, and `0.0.0.0` (what an appliance normally does;
refusing it only pushes users to hardcode a DHCP address). Refused:
anything **globally routable** — that is a password prompt on the
public internet with code execution behind it, and no configuration
meant that. `0.0.0.0` is accepted but returned as
`Exposure::EveryInterface` so the caller must say so: on a host that
later acquires a public address it becomes a public listener with no
config change. Note `100.x` is not automatically Tailscale —
`100.0.0.0/10` and `100.128.0.0/9` are ordinary public space, and
matching the first octet would allow routable addresses.

**TLS is required iff the bind address is not loopback**
(`BindAddr::requires_tls`). `http://127.0.0.1` is already a secure
context in every browser, so loopback development needs no certificate
and loses no browser capability. Everything else carries an admin
password across a wire someone else can be on — and unlike the earlier
tailnet-only design there is no WireGuard underneath a plain LAN, so
here TLS is doing confidentiality work as well as unlocking service
workers, PWA install, and web push. There is no downgrade switch:
`remote::tls` stops the server rather than falling back, because a
silent downgrade leaves the user believing traffic is protected.

Certificates come from a private CA (`scripts/mint-remote-cert.sh`,
idempotent so already-trusted devices keep working) because this
tailnet's self-hosted control server cannot issue them — `tailscale
cert` returns *"your Tailscale account does not support getting TLS
certs"* — and `.internal` is not a real TLD. Two of that script's
constraints are Apple policy and both fail with errors that blame
something else: Safari refuses a leaf valid beyond **398 days**, and
the leaf needs `extendedKeyUsage=serverAuth`. On iOS, installing the CA
profile is half the job; full trust must also be enabled under Settings
> General > About > Certificate Trust Settings.

**Password hashing reverses `remote::token`'s reasoning, deliberately.**
That module argues *against* a memory-hard KDF and is right to: a
256-bit machine token has nothing to brute force. The argument depended
on there being no low-entropy secret. A human-chosen password is
exactly that secret, so `remote::password` uses scrypt. Both modules are
correct for their own input; do not unify them onto one hash.

scrypt rather than argon2 on dependency-hygiene grounds — it is already
compiled in transitively via `age`, so promoting it adds no new
supply-chain surface. Stored hashes are PHC strings carrying their own
algorithm and parameters, so raising the cost or moving to argon2id
later does not invalidate existing passwords.

**The throttle backs off; it never locks out.** A hard lockout on a
LAN-reachable appliance is a denial-of-service handed to anyone on the
wifi: they lock the owner out and risk nothing. Failures buy an
exponentially growing delay capped at 30s — at which point an online
search is dead while an owner who mistyped waits once. The pairing code
in `token` *can* burn itself, because the user can always mint another
at the machine; an admin locked out over the network cannot.

**A bearer token defends against other devices, not against local
code.** `remote-devices.json` is owner-writable and a same-UID process
can write its own record — and can already drive CC's socket directly
without Claudepot, since CC's own boundary there is the Unix user. The
honest claim is narrow: this stops another *device* acting without
credentials, and a *revoked* device acting at all. Do not write docs or
UI copy implying more.

Two failure modes are load-bearing and both were found by an
adversarial review rather than by testing:

- `verify_password` must distinguish "wrong password" from "stored hash
  unusable". A PHC string can parse and carry no hash output
  (`$scrypt$garbage` does), and verifying it returns the same error as a
  wrong password — so `.is_ok()` would tell the owner their correct
  password is wrong, forever, with nothing pointing at the file.
- The pairing code must not consume UUID bytes whose bits are fixed.
  Taking the first eight bytes of a UUIDv4 put the version byte at
  character 7 and cut that position from 26 symbols to 16 — 36.9 bits
  rather than 37.6, and invisible because the code still looked random.

**TOTP (`remote::totp`) is an optional SECOND factor and must never be
the only one.** A TOTP secret cannot be hashed — the server needs it in
recoverable form to compute the expected code — so replacing the
password with it would make the stored credential strictly *more*
valuable than a scrypt hash: reading the file would give working access
instead of a cracking job. It also has no recovery story, and the usual
remedy (printed backup codes) is a password again with worse
ergonomics.

Two details there are load-bearing:

- **Codes are burned.** `TotpState::last_used_counter` is a high-water
  mark, so a code cannot be replayed and an *earlier* still-in-window
  code cannot be used after a later one. Without this a code is live for
  up to `(2 x SKEW_STEPS + 1) x PERIOD_SECS` — 90 seconds — and anyone
  who observes one can reuse it. RFC 6238 §5.2 requires it; most
  implementations omit it.
- **SHA-1 is correct**, not an oversight: it is RFC 6238's default and
  the only algorithm Google Authenticator reads from a plain
  `otpauth://` URI, and it is used inside HMAC where the collision
  weakness does not apply. The implementation is checked against RFC
  6238 Appendix B's published vectors, which is what makes interop with
  real authenticator apps a fact rather than a hope.

Be honest about what it buys: when the client is the phone and the
authenticator app is on that same phone, the second factor is close to
ceremonial — one device compromise defeats both. It defends a credential
used from *elsewhere*. Offer it; do not force it.

**Passkeys / WebAuthn are the better end state**, and they are built:
verification in `remote::webauthn` (ES256, hand-rolled, no attestation),
the ceremony state and origin rules in `remote::passkey`, the four HTTP
steps in `remote::api`. The server stores only a public key, so reading
`remote-config.json` gives an attacker nothing — strictly better than
both a password hash and a TOTP secret.

Four properties there are load-bearing:

- **Registration requires an authenticated session.** Otherwise anyone
  who can reach the page enrols themselves. The password stays the
  bootstrap and recovery credential.
- **The RP ID is derived from the request, never configured.** The same
  appliance reached by `.local` and by MagicDNS gets two credentials
  rather than one broken one — which is correct, since a passkey is
  scoped to an origin by design and the minted certificate covers both.
- **`login/begin` sends an empty `allowCredentials`.** It is
  unauthenticated of necessity, so it must reveal nothing about what is
  registered; `residentKey: "required"` makes the platform resolve the
  credential itself, which costs nothing on the device this is for.
- **A passkey login mints exactly what a password login mints** — the
  same `Device` row, expiry and revocation path. Two kinds of session
  would be two things to remember to revoke.

The prerequisite is **settled, measured on a real iPhone** against this
deployment's private CA (2026-08-23). A privately-trusted certificate on
a tailnet IP gives a full secure context: `isSecureContext` true, a
service worker registering at root scope, `crypto.subtle`, the WebAuthn
API, and `isUserVerifyingPlatformAuthenticatorAvailable()` returning true
for Face ID. Nothing about the private CA degrades the origin.

So the intended auth story is: **password as the bootstrap and recovery
credential, passkey as the day-to-day login.** TOTP stays available and
is expected to go unused — the reasoning above about it being close to
ceremonial when the client and the authenticator are the same phone
applies with more force once that phone can do Face ID instead.

**The origin must be a hostname, not an IP.** WebAuthn's RP ID is
required to be a valid domain, and an IP-address origin has none — so
`https://100.64.x.x:8420` cannot register a passkey however capable the
device is. This is a trap worth naming because
`isUserVerifyingPlatformAuthenticatorAvailable()` answers "does this
DEVICE have an authenticator" and reports `true` on exactly the origin
that cannot use one. The first version of the probe reported only that
flag, which is a green light measuring the wrong thing — the same shape
of error as the port-based `Host` check. It now derives the RP ID the
way a browser would and says so when the origin disqualifies itself.

The minted certificate already covers the `.local` and MagicDNS names,
so reaching the same server by name is enough; no re-mint is needed.

Two further caveats kept separate from the measurement rather than
folded into it:

- iOS grants *standalone PWA install* and *web push* from Safari
  specifically, and the probe was not confirmed to be running in Safari.
  The secure context they depend on is proven; install and push are not.
- Android reaches passkeys through the same standard, but a
  user-installed CA sits in Android's *user* trust store, which Chrome
  honours for browsing. That path is untested here.

Still open at the HTTP layer, recorded so they are not rediscovered:
multiple appliances on one LAN need discovery (mDNS) with a user-set
instance name, and there is no streaming surface yet — when one is
added it must **close** on credential revocation rather than merely
refusing the next request. The rest of that list is done: the throttle
is persisted, every mutation requires an `Idempotency-Key`, and `Host` /
`Origin` are checked in `guard_origin` before any handler runs.

## Wiring it into the desktop app — Settings → Remote

The surface was CLI-only for its whole life: `claudepot remote
{status,set-password,enable,disable,revoke-all,serve}`, with `serve` a
foreground process somebody kept a terminal open for. The GUI now hosts
it, and three decisions hold that together.

**Every verb is `remote::service`, and there is exactly one of it.**
The CLI's command file used to carry real logic — `FilePersist`, the
recovery warning, `enable`'s preflight ordering, `revoke_all`'s refusal
— so a Tauri command written against `claudepot_core::remote` directly
would have reimplemented all of it. That is the `account_service` story
again, and the `revoke_all` refusal is the one that would have hurt:
drop it in a second implementation and the GUI's "Revoke all" reports
`Revoked 0` over a device file whose `revoked_at` marks were already
lost, which reads as "there was nothing to revoke" rather than "every
stolen token is live". `ApprovalHook` and `FilePersist` moved to core
with it. What stays per-caller is presentation.

**The server runs in-process, on a tokio task
(`src-tauri/src/remote_server.rs`).** Not a launchd/systemd daemon, and
the reason is the approval hook rather than convenience: it is armed for
exactly as long as a server is up, and that coupling is what makes it
acceptable to hand a network client the ability to grant a permission at
all. A daemon makes "as long as the surface is up" mean *always*, which
turns a session-scoped capability — code execution as this user — into a
permanent one. That is the trade the peer-inbound grant already refuses
by narrowing temporally.

The cost is real and is **disclosed, not discovered**: quitting
Claudepot stops the remote surface. `RunEvent::Exit` enforces it (not
`ExitRequested`, which can be prevented — stopping the server for a quit
that then does not happen would leave the pane reporting a state nobody
asked for), and the pane says so in as many words.

**Three states, not two, and two liveness fields.** `server.enabled` is
a stored preference that survives a `kill -9`; `approval::store::is_serving`
is the heartbeat. So the pane renders *off* / *enabled, not serving* /
*serving*, and collapsing the middle one is a review finding — a phone
cannot reach a Mac whose preference merely says yes.

`serving` and `running_here` are separate for a second reason: a
`claudepot remote serve` in a terminal sets the heartbeat and is not
ours. A pane with only the first would offer Stop for a process it
cannot stop; one with only the second would report "off" while a
terminal was serving the panel to a phone. Both are asserted in
`RemotePane.test.tsx`, and the three-state test has been watched failing
against a pane that collapsed them.

The pane is **`group: "core"`**, for the reason Retention is: it is
where you revoke a lost phone, and an emergency control you have to go
hunting for is a broken one.

**Quick prompts live inside it**, not in a pane of their own. They are
the chips above the *remote panel's* composer and mean nothing anywhere
else, so a top-level entry put one surface's detail beside Retention and
Health. `QuickPromptsPane` stays its own component — its editor, its CSS
and its behaviour were fine; only its position in the nav was wrong —
and its search terms folded into the Remote pane's `keywords` so ⌘K
still finds it.

**Two chips, not one.** `RemoteServingChip` is the ambient tier for
this surface; `RemoteWindowChip` is Claude Code's `crossSessionInbound`.
Different capabilities with different blast radii — folding them into
one indicator would leave a user who saw it lit unable to tell which
door was open. The new chip reads liveness rather than the preference,
so a CLI-started server lights it too, and renders nothing when nothing
is serving.

**Secret direction is unchanged and worth restating**, because this is
the first surface that takes a password over IPC. It crosses *in* and is
zeroized by `service::set_password` on every path; nothing returns it,
and the device store holds a SHA-256 that `DeviceSummary` does not carry
either. If TOTP enrolment is added later it needs the `key_*_copy`
treatment — the `otpauth://` URI is a secret coming *back*, which
`rules/architecture.md` forbids in the plain shape.

Still absent by design: pairing-code display and QR, and TOTP/passkey
enrolment from the GUI. The endpoints exist; the pane does not drive
them yet.

## The panel — the client that ships at `/`

`panel/` is a self-contained Vite app (its own install; **not** a
workspace member of the Tauri renderer, which carries over 300 `invoke` calls (327 on 2026-09-17)
that mean nothing over HTTP). It builds into
`crates/claudepot-core/src/remote/assets/panel/`, which is **committed**,
so `cargo build` needs no Node. Rebuild with `scripts/build-panel.sh`
after touching anything under `panel/` — nothing else notices, and the
previous bundle ships silently.

The probe moved to `/probe` rather than being deleted. It is not the
product and never was, but it has twice been the only thing able to
answer a question a development machine cannot — it established that a
privately-trusted certificate on a tailnet name yields a full secure
context, and it caught the passkey check measuring the device rather
than the origin.

The endpoints behind it, all in `remote::api` over `remote::panel`:

| Route | Notes |
|---|---|
| `GET /api/sessions` | live PID registry joined to `session_index` on session id; live always, plus the 20 most recently touched |
| `GET /api/sessions/{id}/transcript` | `tail` / `after` / `before` windows, `no-store`. **The secret-bearing endpoint** |
| `POST /api/sessions/{id}/prompt` | the only write that reaches Claude Code, through `peer` |
| `POST /api/sessions/{id}/read` | per-device read mark |
| `GET /api/accounts` | read-only except `…/{email}/activate` |
| `GET /api/sessions/{id}/commands`, `…/commands/{name}` | slash commands as **text**; cwd resolved from the session, never from the client |
| `GET /api/approvals`, `POST /api/approvals/{id}` | the only route that **grants a capability**; alive only while `remote serve` is |
| `POST /api/passkey/{register,login}/{begin,finish}` | register is authenticated, login is not |

Five decisions are worth not re-litigating:

- **A card is titled by the LAST prompt, not the first.** The stored
  `first_user_prompt` names what a session started as, which on a thread
  running for hours is a question settled long ago. `last_user_prompt`
  scans the tail backwards, skipping the text CC writes into the *user*
  role itself (`<command-name>`, `<bash-stdout>`, `<system-reminder>`,
  tool results) — a card reading `<bash-stdout>` would be worse than the
  stale title it replaced. It falls back to the stored first prompt, so
  a session whose tail is all tool traffic still has a name.

  The window **escalates**, 64 KB then 512 KB, and that is not a guess:
  on a real 14 MB transcript the last 64 KB held **zero** user turns
  while 512 KB held three. One large window would pay that on every
  session every poll; escalating pays it only where the cheap read came
  up empty, and the list stayed at 0.34 s. Past the cap the honest
  answer is "no recent prompt in reach" — reading a whole transcript
  every five seconds to name a card is not a trade worth making.
- **Live cards are ordered by when each session last REPLIED.** Not by
  process start — the session you opened first this morning is the one
  you have been working in all day — and not by `last_ts` either, which
  moves on every tool call, so a session grinding through a hundred of
  them sat permanently at the top while having said nothing for
  minutes. `last_reply_ts` is the timestamp of the last assistant turn
  that was *prose*, which is the closest thing a transcript has to "a
  job finished". `last_ts` stays as the tiebreak, so a session that has
  never replied still sorts sensibly. It crosses to the client rather
  than staying server-side: a list ordered by a number the client cannot
  see is a list nobody can check.
- **No `failed` status is synthesised.** The only available signal is
  `SessionRow::has_error`, true of any transcript with one errored tool
  call — routine in a long session. Painting those red would make the
  one colour that should mean "look at this" mean "a command exited
  non-zero". It ships as its own boolean instead.
- **No `stuck`, no `idle_ms`, no tool-call count.** The first two are
  `session_live::LiveRuntime` overlays and `remote serve` has no
  runtime; the third needs a schema migration. All three are absent
  rather than estimated — a card showing a number nobody computed is
  worse than a card without one.
- **Token components travel, not one sum.** `TokenUsage::total()`
  includes cache reads, which dominate by two orders of magnitude: a
  real session here reported 1.8 *billion*. The client renders
  input + output.
- **The transcript endpoint says the text is masked, never scrubbed.**
  `session_live::redact` is explicitly incomplete and knowingly passes
  GitHub PATs and AWS keys. A user who believes a screen is scrubbed
  will screenshot it.
- **Prose renders as markdown; tool output never does.** Claude writes
  markdown, and both transcript viewers used to show it verbatim — a GFM
  table arrived as one run-on line of pipes. Two exclusions are
  deliberate and hold on **both** surfaces (`panel/src/app/Markdown.jsx`
  and `sections/sessions/components/TranscriptMarkdown.tsx`):

  | Case | Renders as | Why |
  |---|---|---|
  | a prose turn | markdown | it is markdown |
  | a tool call or its output | verbatim, mono | a shell comment is not a heading and a glob is not emphasis; the one thing output must be is what the command printed |
  | any turn while a **search** is running (desktop only) | highlighted plain text | `highlight` marks matches by splitting the raw string; a match you can find beats a bold you can read |

  Neither renderer enables `rehype-raw`, so embedded HTML is escaped
  text — the input is model output quoting arbitrary files. Images
  render as their alt text rather than being fetched. Links differ by
  surface on purpose: the panel opens a new tab, the desktop goes
  through `ExternalLink` and the OS opener, because a bare `<a href>`
  inside a Tauri webview navigates **the application itself** away with
  no back button.

  **Mermaid runs with `htmlLabels: false`, and that is load-bearing.**
  By default mermaid puts labels in a `<foreignObject>` of HTML
  containing **unclosed `<br>`**, so the SVG it returns is
  HTML-flavoured rather than well-formed XML. Both renderers parse it
  with `DOMParser` as `image/svg+xml` — a deliberate guard, not an
  accident — and both therefore failed with *"Opening and ending tag
  mismatch: br line 1 and p"* on any diagram whose labels used `<br/>`.
  Measured on three real diagrams: the flowchart and the state diagram
  failed, the sequence diagram drew, and the difference was only that
  the first two had `<br/>` labels. On the desktop it bit twice —
  `sanitizeSvg` strips `foreignObject` outright, so even a diagram that
  parsed would have rendered with its labels missing. Loosening the
  parse to `text/html` would also have worked and is the wrong trade;
  turning HTML labels off fixes both surfaces at the source, and
  `<br/>` still breaks a line because mermaid emits tspans for it.

  **The desktop's diagram styling is a runtime `<style>`, and the
  release CSP refused it.** Mermaid puts every rule it draws with —
  fills, `fill: none` on edge paths, `text-anchor: middle` on labels —
  in a `<style>` element inside the SVG, inserted when the diagram
  mounts. `tauri.conf.json` grants `style-src 'unsafe-inline'`, and in
  dev that is what runs. In the **release bundle** Tauri hardens the
  policy on its own: at build time it stamps a nonce on `index.html`'s
  inline `<style>` and at runtime appends `'nonce-…'` to `style-src` —
  and a nonce or hash source makes browsers **ignore `'unsafe-inline'`**
  (CSP Level 3). So the installed app, and only the installed app, drew
  every diagram with SVG defaults: black node fills, edge curves filled
  black into crescents, labels anchored `start` so their right halves
  poked out of the boxes. Nothing errored; the render succeeded and the
  stylesheet was silently dropped. Reproduced in Playwright WebKit by
  adding one `'nonce-…'` to a harness's `style-src` (the console says
  *"Refused to apply a stylesheet because its hash, its nonce, or
  'unsafe-inline' does not appear in the style-src directive"*), and
  the same page drew correctly without it.

  `dangerousDisableAssetCspModification: ["style-src"]` keeps the
  written policy in force for styles; `script-src` stays under Tauri's
  nonce/hash hardening, which is where that protection matters.
  `MermaidBlock.test.tsx` locks both halves of the config, since either
  one alone is a silent no-op in release and CI never runs the release
  webview. The panel is unaffected: `remote::server` writes its own
  header with no nonces. The general lesson: **dev does not run the
  release CSP**, so any runtime `<style>` injection — a library's, not
  only mermaid's — has to be checked against the built app, not
  `tauri dev`.

  Checked on the real pipeline, both ways. `pnpm tauri build` leaves the
  processed HTML at
  `target/release/build/claudepot-tauri-*/out/tauri-codegen-assets/*.html`,
  brotli-compressed; with the flag its `<style>` is bare, and with the
  flag stripped it reads `<style nonce="__TAURI_STYLE_NONCE__">` — the
  token the runtime turns into the `'nonce-…'` source. Two traps in
  getting there: a plain `cargo build --release -p claudepot-tauri`
  embeds **no** frontend at all, because `generate_context!` gates
  embedding on the `custom-protocol` feature that only the Tauri CLI
  enables; and there is no `.html` to find until it does.

  **The failure says why.** `Mermaid.jsx` captured the reason and then
  rendered a generic sentence, so every failure looked identical from
  outside — which is how one cause (oklch, see below) was fixed while
  this one went on producing the same message. The reason is now
  appended to the notice.

  **`clusterBkg` is the warm page colour, not `--sf`.** `--sf` is the
  CARD colour and in light mode it is pure white, which painted every
  subgraph a stark white box on the diagram's own `--sf2` container and
  read as a rendering fault rather than as depth.

  **A ` ```mermaid ` fence is drawn, on both surfaces.** A diagram in an
  answer *is* the answer; showing its source is showing the wrong
  artifact. Both go through `securityLevel: "strict"` and both
  lazy-import mermaid, which matters far more on the panel: mermaid and
  its diagram packs are larger than the rest of that bundle put
  together, so they are **61 separate chunks** (2026-09-17) served from
  `/panel/chunks/` and the base bundle stays ~426 KB. A thread with no
  diagram pays nothing; a flowchart pulls its own pack, not cytoscape
  and katex.

  The route table for those chunks is **generated** into
  `assets/panel_chunks.rs` by `scripts/build-panel.sh`, because sixty
  hand-written match arms would be wrong within one mermaid upgrade —
  and because a runtime directory walk would hand `remote::assets` the
  traversal surface it exists not to have. Two tests walk the directory
  and fail in **both** directions. The cost, stated rather than buried:
  ~3.8 MB of committed bytes embedded in every binary whether or not the
  remote surface is ever switched on.

- **A card title is markdown-stripped, not markdown-rendered.** The
  opposite of the body, and for the obvious reason: one line has no room
  for a heading, a fence or a list. Every `/execute-plan` session on this
  machine was titled `## User Input ```text …`.

  The rule is `claudepot-core::session::title::derive` (which the panel
  DTO uses) and `deriveSessionTitle` in
  `src/sections/sessions/format.ts` (which the desktop uses, because it
  renders `SessionRow` straight over IPC). Two implementations, pinned by
  `crates/claudepot-core/testdata/session-title-vectors.json` — **both
  run those vectors**, the same arrangement `PriceBook::resolve` and
  `src/costs.ts` have. Change one, change the other, add a vector.

  **Underscores are never touched**, including `__bold__`. The vectors
  caught `__init__ never runs` becoming `init never runs` — the exact
  corruption the module claims to prevent, in the module that claims it.
  Single `*` is left alone for the same reason (`*.log`, `2 * 3`).
- **A home-screen app has no way to reload itself, so the panel ships
  one.** Installed to the iOS home screen there is no address bar and no
  reload button, and the system pull-to-refresh **cannot fire**: the
  shell is `position: fixed; inset: 0; overflow: hidden` and scrolling
  happens inside `.sc` containers, so the document never scrolls. Left
  alone, a standalone panel runs last week's bundle indefinitely with
  nothing on screen saying so — which is how a shipped mermaid fix read
  as unfixed for an afternoon.

  Two halves. `GET /api/sessions` carries `server_version`, and since
  the bundle is embedded in that binary with `include_bytes!` the
  server's version **is** the bundle's — they cannot disagree. The
  client records the version it booted with and shows a one-line
  tap-to-reload bar when a later poll reports a different one. A server
  too old to send the field leaves it null and nothing ever goes stale,
  so the check fails off. Settings → This device → Reload is the manual
  half, for the case you would not notice.

  It is a **tap, never an automatic reload**: reloading mid-sentence
  loses the composer, and "the app updated itself while I was typing" is
  a worse surprise than a bar. `location.reload()` is enough — the panel
  is `no-store`, so a load always fetches current bytes and there is no
  cache to bust.
- **Usage figures come from `usage-snapshot.json`, and something has to
  write it.** The panel renders that file, never a live `/usage` call —
  so on a machine reached only through `remote serve` there were no
  usage figures at all until the desktop app had run, and a window
  added to the schema stayed invisible until someone opened the GUI.
  `claudepot usage refresh` writes it from the CLI. Two consequences
  worth knowing: an **older** desktop build rewrites the file on its
  five-minute tick and silently drops fields it was compiled without —
  which is exactly how a shipped `scoped` window read as "Anthropic
  doesn't send it" — and the snapshot is the reason a figure can be
  stale without anything on screen saying so, hence the `usage as of`
  line on every row.
- **Tool calls fold by default, and folding is not hiding.** Measured
  across five real sessions on this machine, **59–91% of transcript
  rows were tool ticks** — so listing each one turns a 390px column
  into a wall of ticks with the conversation scattered through it. A
  run of two or more collapses to one `N tool calls` row that expands
  into exactly what it replaced; a run of *one* is left alone, because
  a row reading "1 tool call" is strictly worse than the tick, which at
  least names the tool. An errored call is counted on the folded row —
  grouping must not hide the one thing in a run worth looking at.
  Settings → Appearance → Tool calls switches it off. The preference is
  `localStorage`, not server state: this is how one device likes to
  read, and a phone and a laptop pointed at the same Claudepot are
  allowed to disagree.
- **There is no projects surface at all; accounts are writable, and the
  earlier note here was wrong about why.** The projects tab, its screen
  and `GET /api/projects` were deleted rather than left read-only: the
  tab listed what the sessions list already names on every card, and a
  route nothing calls is surface with no reader. A project *move* was
  never offered and still is not — it rewrites path-keyed CC state
  outside the project directory behind a rollback journal, and a
  half-applied one leaves Claude Code pointing at a path that is gone.

  The account half claimed a swap "either fails while CC is running or
  bypasses the keychain-drift guard". `force` is consulted in exactly
  two places in `swap::switch_inner`, **both the live-session gate**.
  The drift check runs unconditionally and is self-healing, so it is
  never what a remote caller skips.

  What a remote caller can skip is the live-session gate, and that gate
  is about **correctness, not security**: a running CC holds its refresh
  token in memory and overwrites the keychain on its next refresh,
  silently reverting the swap. A revert nobody is at the machine to see
  is the worst outcome, so `POST /api/accounts/{email}/activate`
  defaults to the gated `switch` and answers **409 `live_session`**;
  `force` exists but the phone must ask for it and is told what it
  costs. Auto-rotation has always called `switch_force` unattended on a
  timer, so a human tapping a phone is strictly more supervised than
  what already shipped.

  **The CLI slot only.** `cli` and `desktop` are independent nouns and
  `.claude/rules/architecture.md` says never to couple them, so the
  panel moves the first and never the second — Claude Desktop's slot is
  switched at the machine. `swap::switch` touches CC's keychain item and
  nothing of Desktop's (every `Desktop` mention in that module is
  Windows process *detection*, so a running Desktop is not mistaken for
  CC). Reading the swap code proves today's behaviour; the lock that
  matters is `no_route_can_switch_the_desktop_slot`, which fails the day
  a desktop route appears — and has been watched failing against a
  planted one. `ActivateRequest` carries no `desktop` field either, so
  the body is not a way in. The UI names the slot for the same reason:
  a bare "Use" next to a Desktop chip invites the reader to assume it
  moves both.

  Addressed by **email**: that is this domain's identity for an account,
  and a uuid on the wire is an internal identifier the panel would then
  have to render or hide. Resolution is **prefix matching**, the same as
  everywhere else, because the sequence is shared —
  `account_service::activate_cli` is the one implementation of
  resolve → reconcile → compare → swap, and both `claudepot cli use` and
  the endpoint call it. It exists because there were briefly two, and
  the second had already dropped `resolve_email`: a prefix that worked
  at the keyboard answered "account not found" from the phone. What
  stays with each caller is presentation — the CLI's split-brain warning
  and the panel's inline conflict copy say the same thing in different
  registers. Registering, removing and verifying accounts
  stay at the machine — they need credentials the panel never sees.

**Three steps, and only the last restructures.** `data-bp` is written
from `useBP`'s ResizeObserver on the panel's own shell, so one
observation drives type, gutter and layout together — and a 390px split
view renders as a phone regardless of how wide the display is. A
container query cannot express this: `@container` matches DESCENDANTS of
the container and never the container element itself, and the panel *is*
the element that steps.

| | Width | Navigation | Layout |
|---|---|---|---|
| `sm` | ≤479px | bottom tabs | one column; the thread covers the list |
| `md` | 480–899px | bottom tabs | one column, wider gutter and larger type |
| `lg` | ≥900px | left icon rail | list and thread side by side |

The attribute was pinned to `"sm"` for the whole of the panel's life
before this, which left the `md` and `lg` blocks in `ds-tokens.css` as
dead CSS — the app rendered at the phone floor on every screen it was
ever opened on.

Three consequences follow from `lg` and each was a bug at least once:

- **Every branch of `Panel` returns a `Shell`.** `useBP` observes
  `ref.current` in an effect keyed on the ref OBJECT, which is stable, so
  the effect runs exactly once after the first commit. A boot screen
  rendered outside the shell left `ref.current` null at that moment and
  the observer was never attached — reproducing the pinned-to-`sm`
  failure through a different mechanism.
- **The thread in a pane has no Back.** It never covered the list, so a
  chevron there would undo a navigation the user did not make. Tapping a
  rail tab does not clear the open thread either, for the same reason it
  does clear one on a phone: there the tab would light up under a
  transcript still covering it, here it hides nothing.
- **The list says which row is open.** Only at `lg` — `openId` is passed
  as null below it, because a list nobody can see does not need a
  selection. A history row takes the accent wash; a live card takes an
  inset ring instead, since it may already be carrying the live glow and
  two backgrounds on one card read as a rendering fault.

**The back gesture closes the thread; it used to leave the app.** The
panel kept no history at all — one entry for its whole life — and iOS
Safari's edge swipe and Android's back both drive session history, so
the gesture every phone user reaches for first was pointed at the exit.
Opening a thread now pushes an entry (`panel/src/app/Panel.jsx`); a
`popstate` listener is the single closer, so the chevron, a tab tap, a
swipe and the OS gesture cannot diverge — anything that closed the
thread without popping would leave the entry behind and silently eat the
next Back.

Four things about it are load-bearing:

- **Only the thread pushes.** Tabs are lateral, not deeper: the chevron
  model already says the transcript is the one place you go *into*, and
  an entry per tab tap would make Back walk backwards through tabs
  instead of leaving.
- **The URL never changes** (`pushState(state, '')`). `remote::assets`
  serves the panel at `/` with no client router, so a path only the
  client understands would 404 on reload.
- **It fails off.** `historyNav()` returns null where the History API is
  not usable, and the thread still opens and still closes by chevron —
  exactly as before. That is not hypothetical: the render check aliases
  `window` to Node's `globalThis`, which has neither `history` nor
  `addEventListener`, so the unguarded version threw inside a mount
  effect, React unmounted the whole tree, and **all seven** passes went
  blank — including sign-in, which never opens a thread. The guard is
  also why the check now asserts the entry: a feature that disables
  itself is green either way.
- **The in-transcript swipe is a second thing, not the same thing**
  (`panel/src/app/gestures.js`). It complements the OS gesture rather
  than duplicating it — the screen edge is already spoken for in a
  browser tab, and a standalone home-screen app may have no OS gesture
  at all. It routes through the same `onBack`, so it pops the same
  entry. A drag that starts inside a horizontally scrollable element
  with room left to scroll *that* way is not a swipe: a wide code block
  or table is exactly the thing a reader drags sideways, and stealing
  that would be worse than having no swipe. Checked against live
  `scrollLeft`, not the mere presence of overflow, or one wide table
  would disable the gesture for the whole conversation.

**Offline holds the message; it does not refuse it.** The composer used
to disable itself when the host stopped answering, which is honest and
throws away the one thing a phone is good for — you thought of it on the
train. `claudepot-core` is not involved: the queue is
`panel/src/app/outbox.js`, localStorage, client-side.

Four properties, each of which is why it is a module rather than
component state:

- **It survives leaving the thread**, and the reload. A queue held in
  `Thread` dies on Back, which makes "sends when the Mac is back" false
  the moment the user navigates — and an iOS home-screen app is evicted
  from memory whenever the OS likes.
- **The drain is gated on the SESSION, not on the host.** Live and
  addressable, not merely reachable. A session that ended while you were
  offline keeps its queue and waits; posting into a recycled pid is the
  one outcome worse than not sending.
- **The idempotency key is minted at enqueue and travels with the
  entry.** A drain that sends and then loses the answer replays as the
  same intent. Minting at drain time would make a retry a second
  message, which is the only way this feature could send something
  twice.
- **A refused entry stops retrying and says why.** Without that a
  permanently-refused message is re-sent on every poll — a failing
  request every four seconds, forever, from a phone in a pocket.
  `OfflineError` is deliberately not treated that way: "the Mac is not
  answering" is the state the queue exists for. Every entry is
  cancellable from the row it renders on, because otherwise the only way
  to stop one is to be elsewhere when it fires.

**What the design proposed and the panel does not do**, recorded so it is
not rediscovered as an omission:

- **Send does not become interrupt while Claude streams.** There is no
  interrupt: CC's peer inbox takes `user` plus `rename` /
  `notify_when_idle` / `peer_idle_notice` / `peer_message_status`, and
  `TIOCSTI` is refused by current macOS with EACCES even on a pty the
  caller owns. The design's prototype `interrupt` completes its own mock
  stream. A button here would do nothing and say it did something.
- **No Projects tab and no `+` new session.** The first was deleted
  rather than left read-only (see below); the second means spawning an
  interactive session, which `.claude/rules/architecture.md` puts outside
  this product on purpose.
- **No device-pick pairing screen and no push transport.** Pairing is
  password + passkey, which is a different and stronger flow than the
  design's mock; push was flagged as unbuilt by the designer too.

**Two pickers, one sheet.** Slash commands sit behind `/` and quick
prompts behind `…`, and they share `PickerSheet` — position, z-index,
the filter field, the empty and failure states — because a second copy
of that chrome is a second place for the fixed-position rule to drift.
They differ in exactly one way, deliberately: `/` **stages** and `…`
**sends**. A slash command expands to thousands of words and deserves a
look before it goes; a quick prompt is a short phrase its owner wrote so
it could be fired without ceremony.

The quick prompts used to be a scrolling chip row above the composer,
which cost a line on every thread and showed about four before running
off the edge. `…` renders only when there is something behind it.

**Answering without opening** (`remote::panel::ask`) reads exactly one
shape: an unanswered `AskUserQuestion` tool call, whose input carries the
question and its offered choices. Permission prompts are deliberately not
read — a peer message cannot grant an escalation, CC calls trying to do
so *permission laundering*, and a session asked to approve its own
pending prompt is expected to refuse. Tapping a chip sends the label as a
**prompt**; whether an arriving message can resolve a tool call the
session is blocked on has **not been measured**, so the UI says "handed
off" and leaves the question on the card. See the
`pending AskUserQuestion shape` row in `crates/xtask/cc-upstream-watch.md`.

**Sending a slash command** (`claudepot-core::cc_commands`) is possible
only as text, and the panel does the expansion. CC's inbox dispatches
with `skipSlashCommands: true` and its own predicate is
`startsWith("/") && !skipSlashCommands`, so `/audit-fix` otherwise
arrives as nine characters of prose. That flag is hardcoded at every
injection site — it is not a setting, and no permission changes it.

**Expanding here is what CC does there.** CC expands a command at the
input layer and dispatches the *expansion* with `skipSlashCommands:
true`, keeping the original only in `preExpansionValue` so the
transcript can still show what was typed. This is the same step, one
layer earlier — not a way around anything.

Three things it deliberately is not:

- **Not "running the command".** `allowed-tools` does not travel (272 of
  732 command files on the reference machine declare one), nor does
  `model` (40). Invoked properly the command runs under its own
  restriction; sent as text it runs under the *session's*, which is
  generally **wider**. `CommandSpec::restricts_tools` exists so every
  surface says so, and the panel says it on the row and again on the
  staged chip.
- **Not a client-chosen directory.** The cwd is resolved server-side
  from the session id. A path parameter would let one authenticated
  device enumerate `.claude/commands` anywhere on the disk; a session id
  can only name a directory CC is already working in. `CommandSpec.path`
  is `#[serde(skip)]` for the same reason — there is no path for a
  client to hand back, so there is no traversal to defend against.
- **Not one tap.** The picker **stages**; only Send sends. These bodies
  run to thousands of words and some dispatch subprocesses, so the last
  thing between a mistyped filter and a 14,000-word instruction landing
  in a live session is a deliberate press.

Plugin resolution reads `installed_plugins.json`, which records one
entry per *installation* — a plugin in eleven projects appears eleven
times, at eleven paths, possibly at eleven versions. Entries are
deduplicated by plugin name with a project-scoped install beating a
user-scoped one, so the picker offers the same version the session would
actually run, once.

**Approving from the phone** (`remote::approval`) is the one thing on
this surface that grants a capability rather than reading or messaging,
and it does **not** contradict the paragraph above. It uses a different
door: Claude Code's `PermissionRequest` hook, which CC fires *before*
drawing a prompt, in a process CC started itself. No peer message, no
keystroke injection, no laundering — the reasoning in `panel::ask`
stays correct and stays enforced.

Five properties hold it together:

- **Silence is the fall-through.** CC's decision union is `allow` or
  `deny` and has no "ask" arm, so a hook that prints nothing leaves the
  normal prompt to be drawn at the machine. Every failure — surface off,
  dead server, corrupt file, unparseable payload, nobody holding the
  phone — degrades to *exactly today's behaviour*. That is what makes
  the feature safe to add at all: the worst case is walking to the
  machine.
- **It is armed only while `remote serve` is up.** The hook is installed
  on start and revoked on stop (SIGINT **and** SIGTERM — `kill` and
  every process supervisor send the latter, so handling only Ctrl-C
  leaves the entry behind, measured). An install that never turns the
  remote surface on is never asked anything.
- **The runtime gate is the half that holds.** `server.enabled` is a
  stored preference, not liveness — it stays true after a `kill -9`. So
  the server heartbeats every 5 s and the hook believes the heartbeat,
  not the preference. Without it a killed server would leave every
  permission prompt on the machine pausing for the full wait with
  nothing able to answer.
- **The wait ends before CC's does.** CC clamps a hook timeout to
  `UQ_ = 300_000` ms and *kills* the process at it — and a killed hook
  blocks the tool call, the one outcome that does not fall through. So
  `WAIT` (110 s) sits under `HOOK_TIMEOUT_SECS` (120 s) sits under the
  clamp, and there is a test asserting the ordering.
- **One writer per file.** A request and its decision are two files, not
  two fields of one: the hook writes only the request, the server only
  the decision. Atomic rename is crash-safety, not concurrency-safety —
  the same confusion that lost a write in `remote-read-state.json` — and
  these writers are in different processes, where a process-local mutex
  buys nothing. The split removes the race instead of trying to win it.

Be exact about what this widens. Before it, a stolen bearer token could
read transcripts and inject text that CC would refuse to treat as
approval. With it, that token can approve a tool call — arbitrary code
execution as this user, which is what the admin password was always
guarding. It does not weaken the password boundary; it does mean the
boundary now has less behind it in reserve. The `args` exec form is used
so the binary path never reaches a shell parser, the decision is refused
for any id with no live request, and the argument is redacted and capped
on the way *in* so a secret in a command line never reaches the file.

The hook is a **hidden** CLI verb (`claudepot hook permission-request`)
because CC invokes it, not the user; `verify-docs` exempts
`#[command(hide = true)]` verbs from the README on exactly that
reasoning, and tests the exemption in both directions.

**It has its own switch: `approvals_enabled`.** It was hard-wired to
the server's lifetime — starting the server installed the hook, so
wanting the panel at all meant taking the one capability that grants
rather than reads. Those are different questions and are now two:
Settings → Remote has the toggle, `claudepot remote approvals on|off`
is the CLI, and `remote status` reports it. Off, the panel still lists
sessions, reads transcripts and sends prompts; a permission prompt is
drawn at the machine exactly as it was before this feature existed.

Two things about it are load-bearing:

- **The check is in `store::gate`, at runtime, not only at install.** A
  hook entry outlives the process that wrote it — that is the premise
  the whole module is built on and why the runtime gate exists at all —
  so a crash, a hand-edit, or a second Claudepot must not leave the
  capability on after the toggle went off. Uninstalling from
  `settings.json` is tidiness (`reconcile_approval_hook`, shared by
  `ApprovalHook::arm` and the GUI's live toggle); the gate is the line
  that holds. `gate_from` splits that judgement from the reading of the
  config so all eight input combinations are testable without mutating
  `CLAUDEPOT_DATA_DIR`, which every other test in the binary races.
- **It defaults to `true`, unlike `server.enabled`.** The asymmetry is
  deliberate: `enabled` defaults off because a surface that switches
  itself on at install is not a feature, whereas this field is only ever
  read on a machine whose owner has already turned the surface on and
  set a password. Defaulting it off would silently withdraw a working
  capability on upgrade. `#[serde(default)]` on a bool gives `false`, so
  this needs its own helper — and its own test, since losing the helper
  is a silent behaviour change rather than a compile error.
