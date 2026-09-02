#!/usr/bin/env bash
# preflight.sh — run the CI gate locally before you push.
#
# WHY THIS EXISTS
# `cargo test` / `cargo clippy` on macOS is NOT the CI gate. CI also runs
# grep-based architectural guards (scripts/repo-invariants.sh) that no
# cargo/pnpm command covers, and a newer clippy on Linux catches lints a
# macOS-local clippy misses. In v0.1.53 a guard failure sailed past a
# clean local run and only turned up red in CI, mid-release. This script
# runs the SAME checks CI does so that doesn't happen again.
#
# It mirrors .github/workflows/ci.yml's `Format / Clippy (Linux)` +
# frontend jobs. It does NOT reproduce the cross-platform test matrix or
# the Linux-specific clippy toolchain — only CI (or a PR) can. Treat a
# green preflight as necessary, not sufficient: it catches the cheap,
# common failures locally; the PR/CI run is still the source of truth.
#
# RELEASE ORDER (what "doing it right" looks like)
#   1. Feature work on a branch → open a PR. CI validates the full
#      matrix + guards BEFORE anything reaches main. Fix red on the PR.
#   2. Merge the green PR → main stays green by construction.
#   3. On a clean main: run `bump`, fill CHANGELOG.md, commit the bump.
#   4. Tag vX.Y.Z → push tag. The pre-push hook validates Linux+Windows;
#      only use --no-verify if a validator host is down AND CI already
#      proved this exact SHA green.
#   5. release.yml builds + signs installers → smoke-test a packaged
#      artifact (codesign/spctl + launch), then announce.
#   Run THIS script before every push in step 1.
#
# Usage:
#   scripts/preflight.sh           # full gate
#   scripts/preflight.sh --rust    # skip the frontend (pnpm) checks
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

rust_only=0
[ "${1:-}" = "--rust" ] && rust_only=1

step() { printf '\n\033[1;36m▶ %s\033[0m\n' "$1"; }
ok()   { printf '\033[1;32m✓ %s\033[0m\n' "$1"; }

# Mirror ci.yml's package set exactly (note: includes xtask).
step "rustfmt --check"
cargo fmt --check -p claudepot-core -p claudepot-cli -p xtask
ok "formatting"

step "clippy --all-targets -D warnings"
cargo clippy --all-targets -p claudepot-core -p claudepot-cli -p xtask -- -D warnings
ok "clippy"

step "CC-parity fixtures"
cargo xtask verify-cc-parity
ok "cc-parity"

step "architectural invariants (scripts/repo-invariants.sh)"
bash scripts/repo-invariants.sh
ok "invariants"

step "workspace tests"
# Isolate the data root for EVERY test binary. paths.rs's cfg(test) guard
# is per-crate: it covers claudepot-core's unit tests only. Integration
# tests and other crates' tests link core with cfg(test) OFF and would
# otherwise resolve the developer's real ~/.claudepot — which is how a
# test once destroyed a live sessions.db. The runner is the one seam that
# covers all of them at once. (repo-invariants.sh guard 5 asserts this.)
#
# CLAUDE_CONFIG_DIR isolates *Claude Code's* directory for the same
# reason, and it is the one that bites on a developer machine rather
# than in CI. `remote_serve_e2e` resolves it through
# `panel::ListContext::live`, so with only the data root isolated the
# suite pointed an EMPTY index at a REAL ~/.claude: `list_all_sessions`
# then cold-built the index by parsing every transcript on the machine
# — 3.87 GB here — while 24 concurrent tests contended on the fresh
# database. Measured: 61.58 s and a spurious failure
# (`a_transcript_for_an_unknown_session_is_not_found` got 500, because
# a locked index surfaces as `internal`), against 7.62 s and a clean
# pass once both are isolated. CI never saw it because a CI runner has
# no transcripts to parse.
CLAUDEPOT_TEST_DATA_DIR="$(mktemp -d)"
CLAUDEPOT_TEST_CC_DIR="$(mktemp -d)"
mkdir -p "$CLAUDEPOT_TEST_CC_DIR/projects"
trap 'rm -rf "$CLAUDEPOT_TEST_DATA_DIR" "$CLAUDEPOT_TEST_CC_DIR"' EXIT
CLAUDEPOT_DATA_DIR="$CLAUDEPOT_TEST_DATA_DIR" \
CLAUDE_CONFIG_DIR="$CLAUDEPOT_TEST_CC_DIR" \
  cargo test --workspace
ok "rust tests"

if [ "$rust_only" -eq 0 ]; then
  step "frontend typecheck + build"
  pnpm build
  ok "frontend build"

  step "frontend tests (vitest)"
  pnpm test
  ok "frontend tests"

  # Real-app geometry for Global → Config → Env Variables, measured over the
  # dev MCP bridge. vitest runs on jsdom, which has no layout engine, so
  # nothing above this line can observe that an element renders at zero
  # pixels — which is exactly how that pane shipped with its entire editable
  # list invisible. Needs `pnpm tauri dev` running; skipped, not failed, when
  # it isn't (exit 2 means "could not run").
  #
  # `--self-test` first forces the pane to 0px and fails if the
  # assertions DON'T fire. Since CI can never run this guard, a run
  # that merely reports "ok" tells you nothing about whether the guard
  # still works; this makes every local run also a run of the guard
  # against a known-bad layout. The pure `evaluate()` half is covered
  # in CI by `scripts/check-envvar-layout.test.mjs`.
  step "env-vars pane layout (live app)"
  layout_rc=0
  node scripts/check-envvar-layout.mjs --self-test || layout_rc=$?
  case "$layout_rc" in
    0) ok "envvar layout" ;;
    2) printf '\033[1;33m• skipped — app not running (pnpm tauri dev)\033[0m\n' ;;
    *) exit 1 ;;
  esac
fi

printf '\n\033[1;32m✓ preflight clean — safe to push\033[0m\n'
