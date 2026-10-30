# Working notes

Long-form reasoning split out of `AGENTS.md` — first on 2026-09-17, and
again on 2026-10-02, when the remaining per-feature sections and the
data-dir inventory moved here.

`AGENTS.md` is `@`-included into every Claude / Codex / Gemini session,
so its length is a per-session cost; these files are read on demand.
The division is deliberate and one-way:

- **`AGENTS.md` states the rule** — one or two lines, enough to follow
  or to notice you are about to break it.
- **A note carries the argument** — the measurement, the version it was
  verified against, the alternative that was tried and reverted, and
  the failure that produced the rule.

So a note is not optional reading before changing the area it covers.
The rule tells you *what*; only the note tells you *why the obvious
alternative is wrong*, which is the part that keeps a settled decision
from being re-litigated or quietly undone.

| File | Covers |
|---|---|
| `data-dir.md` | every file under `~/.claudepot/`, its owner and its contract. **Gated**: `cargo xtask verify-docs` fails when a store in the source is missing from it |
| `gui-shell.md` | what was measured in the Tauri renderer, three optimisations written, measured and reverted, the index loop's 34 GB, plus the section registry, event channels and the ⌘K palette |
| `cc-integration.md` | the surfaces that reach into Claude Code's own settings, hooks and sockets: retention, permission grants, peer messaging, env variables, the global-config resolver and the settings-file mutation boundary |
| `remote-control.md` | the LAN appliance model, TLS and the private CA, password / TOTP / passkey reasoning, and every design decision inside the phone panel |
| `relay.md` | the subscription relay: separation from Remote Control, gateway mode as measured, the CC-active-account rule, the lease, pairing, per-request binding, streaming and cost |
| `core-modules.md` | pricing, the env secret vault, auto-rotation, proactive token refresh, and the corpus detectors |
| `i18n.md` | the three catalogs, and each rule that was learned by shipping the bug |
| `test-gates.md` | every gate in `## Test`, and the failure that had already shipped green when it was written |
| `assets-and-release.md` | the icon pipeline, the screenshot fixture, and the release validators |
| `web.md` | the claudepot.com app under `web/` |

Two conventions:

- **Dated claims stay dated.** A CC version pin in a note is a
  historical statement and remains true; it is not evidence about the
  installed binary. `cargo xtask cc-drift` prints the gap.
- **`dev-docs/` is gitignored** and is not part of a clone. A pointer
  into it resolves only on a machine that already has it — these notes
  are the tracked half.
