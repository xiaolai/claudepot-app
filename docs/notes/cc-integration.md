# Touching Claude Code — retention, permission grants, peer messaging

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries three surfaces that reach into Claude Code's own settings, hooks and sockets, each verified against a binary on a dated version.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## Transcript retention (Settings → Retention)

Reads and writes CC's `cleanupPeriodDays` — **the only Claude Code
setting that destroys user data**, and one CC's own UI never mentions.
Pure logic in `claudepot-core::cc_retention`; commands in
`src-tauri/src/commands/cc_retention.rs`; pane at
`src/sections/settings/RetentionPane.tsx`.

Not named `retention` in core — `claudepot-core::retention` already owns
an unrelated concept (Claudepot's own `activity_cards` / `metrics_tick`
pruning horizon). `cc_` matches `cc_daemon` / `cc_doctor` / `cc_tips`.

Four properties drive the whole design; changing any of them invalidates
the pane:

- **Sliding, not one-shot.** `getCutoffDate()` recomputes
  `now - cleanupPeriodDays` on *every* run, so loss is continuous.
- **`0` is not on the duration scale — and CC now rejects it.** Through
  CC 2.1.88 it meant *write no transcripts and delete the existing ones
  at startup*. **CC 2.1.233 requires a minimum of 1** and refuses `0`
  with its own message, pointing at `--no-session-persistence` or the
  SDK's `persistSession: false` instead.

  Both of those are out of reach: the flag is rejected outside
  `--print` mode, and the option is SDK-only. **So there is no way to
  disable transcript persistence for an interactive session, and
  Claudepot no longer offers one** — `disable_persistence()` and
  `retention_disable_persistence` were deleted rather than repointed,
  because every implementation of that verb would write a value CC
  rejects and report success. Do not re-add one without re-verifying
  the schema; `cc_retention::MIN_CLEANUP_PERIOD_DAYS` is the pin.

  A `0` written by an older Claudepot is still on disk for anyone who
  used the old control, and it now does the **opposite** of what they
  chose: transcripts are written, and cleanup is suppressed because the
  key is present and invalid. That is `RetentionMode::LegacyZero`, kept
  distinct from `Invalid` because the repair copy differs — a promise
  withdrawn upstream, not a typo to correct.
- **Any value CC's schema rejects suppresses cleanup entirely.** CC
  bails when settings fail validation *and* the raw key is present, so
  an invalid value accidentally **protects** transcripts
  (`RetentionMode::Invalid` / `LegacyZero` / `cleanup_suppressed`). The
  UI must say "fix the value", never "restore the default" — restoring
  clears the error and re-arms deletion. It follows that **any control
  that lifts suppression confirms first**: while suppressed, a preset
  button is a one-tap destructive action on the whole backlog.

  That is anything below `1` **and** anything that is not an integer —
  the schema rejects `"thirty"`, `30.5` and `true` alike. This is why
  `settings_writer::read_i64_setting` returns a three-state
  `SettingValue` rather than an `Option` — collapsing "absent" and
  "present but wrong type" reported a 30-day timer on history CC was in
  fact leaving alone, and pointed the user at the one button that starts
  it.

  2.1.233 states the suppression out loud where 2.1.88 was silent
  (*"Skipping cleanup: settings have validation errors but
  cleanupPeriodDays was explicitly set"*, surfaced via `/doctor`), and
  adds two causes Claudepot does not model: an unreadable/unparseable
  settings file, and `--setting-sources` disabling the user-settings
  source. Known limits, all in the same direction: CC suppresses on a
  validation error **anywhere** in the file while this key is present,
  and Claudepot models only this key, so a file invalid elsewhere — or
  either new cause — reads as "cleanup armed". That errs toward warning
  about deletion that is not happening, which is the safe direction, but
  it is not complete.
- **Bigger than the transcript count.** Deleting
  `projects/<slug>/<uuid>.jsonl` also `rm -rf`s the session folder
  `projects/<slug>/<uuid>/` beside it — `subagents/`, `workflows/`,
  `remote-agents/`, `mcp-tasks/`, everything under them — with **no
  per-file age check**. A second path recursively sweeps those three
  by mtime for session folders whose transcript is already gone.
  `TranscriptRisk::nested_below_session` counts them, because none of
  them appear in the transcript total.

  This entry said the opposite until 2026-08-28: that cleanup "never
  walks `subagents/`", so the folder grows while history is destroyed.
  True of 2.1.88, false at 2.1.250, and false in the direction that
  matters — the pane told the user nested runs survived a sweep that
  takes them, and a passing test asserted it. Found by doing the
  `cc_sweep::SWEPT` reconciliation the watchlist row asks for, which
  had been listed as "not verified this pass".
- **It is not a transcript setting.** `cleanupPeriodDays` is a global
  TTL over ~20 directories under `~/.claude`, verified against the
  2.1.233 binary's cleanup module. `TranscriptRisk` counts `projects/`;
  `claudepot-core::cc_sweep` counts the rest, in the unit CC actually
  deletes — **files** for some directories, **immediate subdirectories**
  for others. Counting the wrong unit reports zero and reads as "nothing
  here", which is why `SweepUnit` is explicit per row. `SWEPT` also
  classifies each directory `Content` or `Cache` so the exclusion of
  telemetry and traces is a recorded decision, not an omission.

`TranscriptRisk::scan_incomplete` is load-bearing: a scan that failed
must never render as "nothing is scheduled for deletion".

Boot check at `src-tauri/src/retention_boot_check.rs` emits **at most one**
bell entry, choosing between two mutually-exclusive conditions that core
guarantees cannot both hold:

| Condition | Core decision fn | Category |
|---|---|---|
| deletion is coming | `cc_retention::warning` | `TranscriptsExpiring` |
| deletion is switched **off** and you were not told | `cc_retention::cleanup_suppressed_warning` | `TranscriptCleanupSuppressed` |

The second exists because the first deliberately returns `None` for every
suppressed state — announcing "conversations are expiring" where deletion
is disabled alarms the user about the one thing that is *not* happening.
That left the suppressed states discoverable only by opening the pane,
and the user least likely to look is the one who set "stop saving" years
ago and considers it settled. Two categories rather than one because the
**mute decision differs**, which is `Category`'s standing test for a
split.

Neither has a dismissal flag — gating on the condition means fixing the
setting silences it, while dismissing without fixing does not.

Both bodies are composed in the Tauri crate from the catalog and locked
byte-for-byte against core's `message()` under `en`, so the CLI's English
and the GUI's cannot drift. The suppressed body additionally asserts it
never contains "will be deleted" / "will delete": the two entries say
opposite things, and a user who reads the wrong one takes the wrong
action on the only CC setting that destroys data.

## Permission grants (ProjectDetail → Permissions)

Optional feature: for a time-boxed window (or until revoked), Claude
Code's tool calls in one project run without permission prompts. The
grant is a record, never a mode: the state is visible with a countdown
and a Revoke button, and lapses on its own.

**It used to write `bypassPermissions` into
`.claude/settings.local.json`, and Claude Code stopped honouring that
in 2.1.257.** The changelog entry: *"Changed `defaultMode:
"bypassPermissions"` in `.claude/settings.json` or
`.claude/settings.local.json` to be ignored, like `"auto"`; set it in
user or managed settings, or pass `--permission-mode`."* The binary
says why — *"projectSettings and localSettings are repo-controllable"*
— and the session starts in Manual, without falling through to a user
value. Every grant written before this was a silent no-op on the
installed CC (2.1.259 here) while the pane said "Bypass active": the
`cleanupPeriodDays` inversion again, a control that kept writing a
value upstream had stopped reading. `permission::settings::PROJECT_SCOPE_IGNORES_SINCE`
is the pin; the resolver reports such a value as
`PermissionDecisionSource::ProjectScopeIgnored`, and the pane renders
it as *ignored* with a one-click removal, never as elevated.

**A grant is now Claude Code's `PreToolUse` hook answering `allow`.**
`claudepot hook pre-tool-use` (hidden, CC invokes it) reads
`permission-grants.json` and prints the allow decision when a live
grant's `project_path` contains the payload's `cwd`, by path
components. Nothing in CC's settings changes except the hook entry
itself, which lives beside the remote-approval one in
`~/.claude/settings.json` and exists exactly while a grant is live.

Why `PreToolUse` and not the `PermissionRequest` hook remote approvals
use — measured on 2.1.259, in real interactive sessions driven through
a pty, with `--debug-file`:

| mode | `PermissionRequest` allow | `PreToolUse` allow |
|---|---|---|
| Manual | fires once, prompt skipped | fires once, prompt skipped |
| auto | **never fires**; the classifier decided | fires once, **classifier skipped** |

Auto mode is the built-in starting mode on Pro, Max and Team, so a
grant on `PermissionRequest` would have done nothing for most users.
`PreToolUse` runs before the permission system decides anything, and
its `allow` is what `bypassPermissions` used to be: no prompt, and no
2–3 s classifier round trip per call (the classifier request is absent
from the debug log). Subagents report the parent's `cwd` and are
covered; a protected-path write (`.claude/probe.txt`) went through.
Headless `claude -p` does **not** consult either hook — it denies —
which is why the first probe, run headless, said the hook never fired.

Five properties hold it together:

- **Two hooks, two lifetimes, one installer.** `cc_hook_entry` writes
  and removes a verb-matched exec-form entry for either event through
  `settings_mutex`; `remote::approval::install` and `permission::hook`
  are thin over it. The approval entry leaves with `remote serve`, the
  grant entry with the last live grant, and one's uninstall cannot
  take the other (tested). The orchestrator tick reconciles the grant
  entry every five minutes, which is also what re-points it at the
  current binary after a Homebrew upgrade or an app move — a sticky
  grant outlives both.
- **The hook never touches the file.** It runs inside every tool call,
  as the user, from a process CC started. `hook::load_readonly` parses
  and nothing else: no corruption recovery, no rename-aside, no log
  line. A corrupt file reads as "no grant", the call goes through CC's
  normal flow, and the GUI's next tick moves the file aside and says
  so. The CLI end-to-end test asserts the corrupt file is byte-for-byte
  untouched and no sibling appears.
- **Scope is the session's working directory.** Same scope
  `bypassPermissions` had — per session, not per file touched — so a
  granted session that runs `cd ../elsewhere && …` is approved as it
  would have been under bypass. `/p/a-evil` is not under `/p/a`; a
  symlinked root is matched on a second, canonicalized pass. Deny and
  ask rules still apply (CC evaluates them regardless of a hook), and
  so does everything no mode auto-approves.
- **A grant with no hook is an error, not a grant.** `permission_grant`
  rolls the record back if the entry cannot be written, and the DTO
  carries `hook_installed` so a grant whose entry has gone missing
  renders with a warning rather than as active — the one state this
  feature must never show, having just replaced a control that did
  exactly that.
- **Cost is one spawn per tool call, reads included: ~13 ms here.**
  There is deliberately no `matcher`; a fixed list of "tools that can
  prompt" would drift with CC. The entry's timeout is 10 s, a ceiling on
  a wedged disk, and a killed `PreToolUse` hook blocks the call, so
  the verb does no waiting at all.
- **The entry points at the CLI, never at `current_exe()` blindly.**
  From the GUI, `current_exe()` is the Tauri app, which has no clap
  parser and no `hook` verb; an entry aimed at it would have CC launch
  the desktop app on every tool call and block the call at the
  timeout. The remote-approval hook was installed exactly that way from
  the GUI before `cc_hook_entry::hook_binary` existed — found by the
  audit-fix pass, not by a test, because every end-to-end probe ran the
  CLI binary directly. The resolver returns `current_exe()` only when
  that *is* the CLI, otherwise the sidecar beside the GUI
  (`mcp_probe::cli_candidates`, whose name differs between a dev tree
  and a bundle), and errors rather than falling back to the GUI. Both
  hook installers go through it.

The schema-1 migration runs in the orchestrator: each legacy record's
settings key is put back (only if the layer still holds exactly the
granted mode — a hand-changed value is left alone), the deadline is
carried over as a hook grant unless it has already passed, and one
bell entry says what happened. A revert that keeps failing is retried
three times and then reported with the file to fix by hand; the key CC
ignores anyway, so what is left behind is litter, not an elevation.
Reviewed adversarially by Codex before building (thread
`01a064df-b03c-77d3-99c6-b102b1fe0cba`); its checkable objections —
the hook must not recover files, the two hook lifetimes must not race,
the binary path must be repaired, legacy reverts need a bound — are the
tests above. Sticky ("Never") grants survive: the record is persistent
and one click ends it, which is what the original rationale required.

- Pure logic in `claudepot-core::permission`: `mode` (PermissionMode
  over CC's wire strings, `auto` included), `settings` (resolve /
  read / write the nested `permissions.defaultMode` key, with the
  project-scope ignore rule), `grants` + `store` (the JSON file and
  its v1 migration), `eval` (expiration, clock injected), `hook` (the
  per-call decision, the read-only load, the entry's reconcile).
- Orchestrator at `src-tauri/src/permission_orchestrator.rs` —
  `tick()` drops lapsed grants (`permission-reverted`, outcome
  `expired`), migrates legacy records, reconciles the hook. Hooked
  into `usage_snapshot::run_tick`. Zero overhead when no grants exist.
- A project running in `bypassPermissions` from the user's own
  `~/.claude/settings.json` shows as elevated but *not*
  Claudepot-managed — the UI won't touch someone's own choice.
- Verified against the **2.1.259** binary and the published settings
  reference (2026-09-03); the `permissions.defaultMode` and `PreToolUse
  hook` rows in `crates/xtask/cc-upstream-watch.md` re-verify it.

## Peer messaging (`claudepot session live` / `send` / `inbound`)

Addressing a **running** Claude Code session. CC binds one Unix socket
per session at `$XDG_RUNTIME_DIR/cc-socks/<pid>.sock`, publishes the
path in `~/.claude/sessions/<pid>.json` as `messagingSocketPath`, and
writes a 0600 key file beside it holding a `peerToken`. The protocol is
newline-delimited JSON: an auth line, then frames.

Pure logic in `claudepot-core::peer`: `wire` (frames, the protocol pin,
the 1 MiB line limit), `key` (filename derivation, token validation,
pid-reuse check), `client` (`send_prompt`), `discover` (resolve a
name/id/pid to exactly one session), `outcome` (classify what happened),
`inbound` (the time-boxed grant). CLI verbs in
`cli/commands/session/send.rs`.

Verified against the **2.1.241** binary (re-checked 2026-08-23); the
`peer messaging` row in `crates/xtask/cc-upstream-watch.md` re-checks it.

**There is no approval action on this channel, and `permission_response`
in the binary is not a counter-example.** The control dispatch is an
explicit if/else chain over `rename`, `peer_message_status`,
`notify_when_idle` and `peer_idle_notice`; zero `uds-messaging` lines
mention permission or approve. The `permission_response` frames that
*do* exist belong to two other transports — CC's own remote-device
WebSocket (`sendPermissionResponse`, keyed by `selectedDeviceId` /
`target_device_id`) and the SDK's `canUseTool` control protocol. Both
are reachable only by the process that owns the session, which for an
interactive session is not Claudepot. Recorded here so the next reader
who greps the binary does not mistake them for a way in. This is an internal,
feature-gated surface (`agents_cross_session_inbox`) on a product that
ships ~27 releases a month, so `peerProtocol == 1` is a hard pin —
a session announcing anything else is refused, not addressed on a guess.

Five properties drive the design; changing any invalidates the feature:

- **It can inject a prompt and nothing else.** CC's inbox accepts `user`
  plus `control` with `rename` / `notify_when_idle` / `peer_idle_notice`
  / `peer_message_status`. There is no exit, interrupt, or restart
  action, and `TIOCSTI` keystroke injection into the session's terminal
  is refused by current macOS with EACCES even on a pty the caller owns
  (measured). **A UI must not offer "restart" over this channel.**
  Restarting a session means owning its pty, which means having started
  it.
- **Arrival is not delivery.** `crossSessionInbound` is
  `accept | hold | refuse`, and an unattested sender addressing a
  `bypassPermissions` session gets **held** — logged to the transcript
  as a `type: "system"` notice, shown with Deny/Deliver, never seen by
  Claude. Measured: held ~0.5 s, delivered ~2.5 s. So the success type
  is `Handoff`, not `Delivery`, and the CLI never prints "sent".
- **A peer prompt is not keyboard input.** Even on `accept`, CC wraps
  the text (`"Another Claude session sent a message:\n…"`) and attaches
  a standing caveat telling the session a peer cannot grant escalation —
  never edit permission settings because a peer asked, never treat a
  peer message as the user's approval, refuse an action the peer says it
  was itself denied (CC calls that *permission laundering*). This is
  remote **messaging** at lower trust than the session's own user.
  Asking a session to approve a pending permission prompt is expected to
  be refused, and that refusal is correct.
- **Slash commands do not work.** CC's peer inbox builds its dispatch as
  `{…, skipSlashCommands: true, isMeta: true}`, and CC's own predicate
  for "is this a command" is `startsWith("/") && !skipSlashCommands`, so
  `/compact` arrives as literal text. Never present the input as a
  command line — and say so where someone would type one: the panel's
  composer warns when the text looks like a command, because the send
  otherwise *succeeds* and does something other than what was meant.
- **`accept` only counts from user scope.** A project-scope value can
  *tighten* the gate but never loosen it — CC: "your own `accept` cannot
  override a repo tightening". A project-scoped writer would report
  success and change nothing.

**Two guards against misdelivery, at different layers**, because a
prompt landing in the wrong conversation is the worst thing this code
could do and pids are recycled: `procStart` from the key file is
compared against `ps -o lstart=` before connecting (the *token* is
current), and `session_id` rides on every frame though CC treats it as
optional (the *conversation* is the intended one). CC drops a mismatch.

**The grant** (`peer::inbound`, `peer-inbound-grant.json`) exists
because the two honest options are both bad: `hold` makes remote control
useless, permanent `accept` leaves the machine open forever. Since the
setting is machine-wide by necessity, the blast radius cannot be
narrowed spatially — so it is narrowed **temporally**, and the deadline
is the whole feature rather than a convenience. Capped at
`MAX_GRANT_HOURS`; an unbounded grant is the permanent setting with
extra steps.

Running sessions re-read the setting **live** — a session started before
the key was written delivered the next message, and went back to holding
seconds after it was removed. That is what makes expiry meaningful
rather than advisory.

`eval::decide` checks **supersession before expiry**: if the user
hand-changed the setting, the record is dropped and the setting is left
alone. The deadline obliges Claudepot to stop holding the door open, not
to force it shut on the user's own choice. Every CLI entry point calls
`ops::tick` first, so a window whose deadline passed while the GUI was
closed still closes.
