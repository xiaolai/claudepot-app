#!/usr/bin/env bash
# Architectural invariants — grep-based tripwires for rules the type
# system can't enforce. Each mirrors a decision in
# dev-docs/codex-plans/20260515-1130-shared-memory.md.
#
# SINGLE SOURCE OF TRUTH. Both callers run this exact file:
#   - .github/workflows/ci.yml  (Format / Clippy (Linux) job)
#   - scripts/preflight.sh      (the local pre-push gate)
# so "green locally" and "green in CI" can't diverge on these checks —
# which is exactly how a guard failure once slipped past a macOS-local
# run and only surfaced in CI (v0.1.53).
#
# Reports EVERY violation (not just the first) and exits non-zero if any
# fired. No side effects; safe to run anytime from the repo root.
set -euo pipefail

fail=0

# ── 1. Codex rollout parser is a single leaf ─────────────────────────
# The decoder pattern is `"session_meta" =>` in a match arm; it must
# live only in codex_session/. Two parsers drifting in parallel is the
# failure the leaf-module discipline prevents. Plain string occurrences
# (fixtures, comments) are intentionally ignored — they aren't parsers.
violators=$(grep -rnE '"session_meta"[[:space:]]*=>' crates/ --include='*.rs' || true)
unexpected=$(echo "$violators" | grep -v 'crates/claudepot-core/src/codex_session/' || true)
if [ -n "$unexpected" ]; then
  echo "::error::Codex rollout decoder found outside codex_session/:"
  echo "$unexpected"
  echo "  Codex rollout parsing must live in claudepot-core/src/codex_session/."
  echo
  fail=1
fi

# ── 2. Raw text SELECTs confined to redaction-aware paths ────────────
# R9: every read of transcript-content columns must be redacted before
# it crosses an emission boundary. New SELECTs of these columns land in
# a file that redacts at the call site — extend the allowlist below and
# say why, exactly as this message directs.
patterns='SELECT.*user_text|SELECT.*assistant_text|SELECT.*tool_result_text'
violators=$(grep -rlE "$patterns" crates/ --include='*.rs' || true)
# session/search/mod.rs (search_index / _infix / _tool_calls) reads these
# columns only to locate the match and build the snippet; every emitted
# snippet goes through redact_secrets — the same redactor the file's
# search_rows JSONL path already uses — and the snippet_text fallback is
# pre-redacted at rest.
unexpected=$(echo "$violators" \
  | grep -v 'crates/claudepot-core/src/shared_memory/search.rs' \
  | grep -v 'crates/claudepot-core/src/shared_memory/read.rs' \
  | grep -v 'crates/claudepot-core/src/shared_memory/indexer.rs' \
  | grep -v 'crates/claudepot-core/src/shared_memory/schema.rs' \
  | grep -v 'crates/claudepot-core/src/shared_memory/claude_exchanges.rs' \
  | grep -v 'crates/claudepot-core/src/session/search/mod.rs' \
  || true)
if [ -n "$unexpected" ]; then
  echo "::error::Raw text SELECTs found outside redaction-aware paths:"
  echo "$unexpected"
  echo "  Every emission of exchanges.user_text / assistant_text /"
  echo "  tool_calls.tool_result_text must go through redaction before a"
  echo "  UI / export / log / MCP boundary. Extend the allowlist in this"
  echo "  script if adding a legitimate reader, and say why redaction is"
  echo "  handled at the call site."
  echo
  fail=1
fi

# ── 3. No bare matches!() statements ─────────────────────────────────
# A bare `matches!(err, Variant);` returns bool and discards it — the
# test then passes against any variant. Wrap in assert!(matches!(...)).
# The signal is matches!() as a *statement* (trailing `;`); match arms,
# returns, and let-bindings have no trailing semicolon and are excluded.
violators=$(grep -rnE '^[[:space:]]+matches!\(.*\);' crates/ --include='*.rs' || true)
if [ -n "$violators" ]; then
  echo "::error::Bare matches!() statements found:"
  echo "$violators"
  echo "  A bare matches!() discards its bool — wrap in assert!(matches!(...))."
  echo
  fail=1
fi

# ── 4. The Claudepot data root is resolved ONLY via paths.rs ─────────
# Hand-building `$HOME/.claudepot` bypasses BOTH the CLAUDEPOT_DATA_DIR
# override (a relocated data dir is silently ignored) AND the test-
# isolation guard in paths.rs. That combination is how a unit test once
# destroyed a live sessions.db (129 -> 1 sessions, 8131 -> 0 exchanges):
# code reached the real data root without passing the one guarded door.
# Resolve through `paths::claudepot_data_dir()`. Temp-rooted joins in
# tests (`tmp.path().join(".claudepot")`) are legitimate and excluded.
violators=$(grep -rn 'join("\.claudepot")' crates src-tauri --include='*.rs' 2>/dev/null \
  | grep -v '/paths\.rs:' \
  | grep -v 'tmp\.path()\.join' \
  || true)
if [ -n "$violators" ]; then
  echo "::error::Hand-built \$HOME/.claudepot outside paths.rs:"
  echo "$violators"
  echo "  Resolve the data root via claudepot_core::paths::claudepot_data_dir()."
  echo "  A hardcoded path bypasses CLAUDEPOT_DATA_DIR and the test-isolation guard."
  echo
  fail=1
fi

# ── 5. Every `cargo test` invocation isolates the data root ──────────
# `cfg(test)` is per-crate, so paths.rs's test-isolation guard covers
# ONLY claudepot-core's own unit tests. Integration tests under `tests/`
# and other crates' tests link core with cfg(test) OFF and would resolve
# the developer's real ~/.claudepot.
#
# Enforcing that per-FILE is a weak guard (a grep proves the file
# mentions the var, not that every test sets it). The runner is the
# right seam: exporting CLAUDEPOT_DATA_DIR around `cargo test` covers
# EVERY test binary at once, with no ritual to forget. So we assert the
# runners actually do it.
for runner in scripts/preflight.sh .github/workflows/ci.yml; do
  [ -f "$runner" ] || continue
  if ! grep -q 'CLAUDEPOT_DATA_DIR' "$runner"; then
    echo "::error::$runner runs cargo test without isolating CLAUDEPOT_DATA_DIR."
    echo "  paths.rs's cfg(test) guard does NOT reach integration tests or"
    echo "  other crates' tests — they would use the real ~/.claudepot."
    echo "  Export CLAUDEPOT_DATA_DIR to a tempdir around the test step."
    echo
    fail=1
  fi
done

# ── 6. MCP reads stay inside the project confinement ─────────────────
# sessions.db is a CROSS-PROJECT index — on a real machine it holds
# every project the user has ever opened, including work unrelated to
# the repo an agent is running in. The memory server is therefore
# confined to one project (claudepot_core::shared_memory::scope), and
# every read tool must honor it.
#
# Two ways to silently reopen the hole, both greppable:
#   a) hardcoding `project_path_exact: None` in the server (that field
#      IS the confinement — the substring `project_path` filter cannot
#      carry it, since `/x/app` is a substring of `/x/app-old`);
#   b) calling `sms::list_projects` without passing `self.scope.root()`,
#      which enumerates every project the user has by name.
# Shipped in 0.1.54 with all reads unscoped; do not reintroduce.
mcp_dir='crates/claudepot-cli/src/commands/mcp'
if [ -d "$mcp_dir" ]; then
  violators=$(grep -rn 'project_path_exact: *None' "$mcp_dir" --include='*.rs' || true)
  if [ -n "$violators" ]; then
    echo "::error::MCP server disables its project confinement:"
    echo "$violators"
    echo "  project_path_exact carries the cross-project boundary. Derive it"
    echo "  from scope::McpScope::confine_search(), never hardcode None."
    echo
    fail=1
  fi
  violators=$(grep -rn 'sms::list_projects(' "$mcp_dir" --include='*.rs' \
    | grep -v 'self\.scope\.root()' || true)
  if [ -n "$violators" ]; then
    echo "::error::MCP list_projects called without the confinement root:"
    echo "$violators"
    echo "  Pass self.scope.root() — a confined agent must not enumerate the"
    echo "  user's other projects. Directory names are themselves disclosure."
    echo
    fail=1
  fi
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "repo invariants: clean"
