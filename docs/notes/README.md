# Working notes

Long-form reasoning split out of `AGENTS.md` on 2026-09-17.

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
| `remote-control.md` | the LAN appliance model, TLS and the private CA, password / TOTP / passkey reasoning, and every design decision inside the phone panel |
| `gui-shell.md` | what was measured in the Tauri renderer and in `~/.claudepot/`, plus three optimisations written, measured and reverted |
| `test-gates.md` | every gate in `## Test`, and the failure that had already shipped green when it was written |
| `cc-integration.md` | transcript retention, permission grants and peer messaging — three surfaces that reach into Claude Code's own settings, hooks and sockets |
| `i18n.md` | the three catalogs, and each rule that was learned by shipping the bug |
| `assets-and-release.md` | the icon pipeline, the screenshot fixture, and the release validators |

Two conventions:

- **Dated claims stay dated.** A CC version pin in a note is a
  historical statement and remains true; it is not evidence about the
  installed binary. `cargo xtask cc-drift` prints the gap.
- **`dev-docs/` is gitignored** and is not part of a clone. A pointer
  into it resolves only on a machine that already has it — these notes
  are the tracked half.
