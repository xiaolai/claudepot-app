# Claude Code drifts under us — two rules

Claude Code ships **~27 releases a month**, and Claudepot reimplements,
parses, or depends on CC behaviour in ~20 places. Any CC claim more than
a few weeks old is a hypothesis, including the ones written in this repo.

## 1. Verify against the installed binary, never the source mirror

`~/github/claude_code_src` is a third-party mirror pinned at **2.1.88**
and abandoned upstream on 2026-04-15. It is archaeology. Used as a
verification source it is worse than nothing, because it confirms
April's behaviour and reports success.

CC ships as a bun-compiled binary that retains readable JS and string
literals, so it is the authority:

```bash
strings -n 60 ~/.local/share/claude/versions/<ver> | grep '<pattern>'
claude --help | grep -- '--<flag>'
```

## 2. Every CC-facing surface has a watchlist row

The list is `crates/xtask/cc-upstream-watch.md` — it sits next to the
tool because it is the *input* to `cargo xtask cc-drift`, and because
everything in this directory is loaded into every session while a
monthly target list does not need to be.

Adding a module that mirrors CC behaviour without adding a row is a
review finding, the same way an event channel without a subscriber is.
`cargo xtask verify-docs` fails when the table stops parsing or names a
module that no longer exists.

To check whether anything has moved:

```bash
cargo xtask cc-drift            # since the parity pin, changelog via gh
cargo xtask cc-drift --since 2.1.220
```

It reports candidates, not findings — confirm each against the binary
before acting, and record the green rows too.

## Why this is written down at all

`cleanupPeriodDays` is the worked example. CC started rejecting `0`; the
control Claudepot shipped for it kept writing `0` behind a
type-to-confirm gate, which inverted its effect from "delete everything"
to "never clean up, and keep writing". It was wrong for some unknown
part of 145 releases, and every doc comment in the module cited the
mirror.
