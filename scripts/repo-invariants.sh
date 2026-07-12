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

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "repo invariants: clean"
