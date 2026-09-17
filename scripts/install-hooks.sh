#!/usr/bin/env bash
# Install this repo's git hooks for the current clone. Idempotent.
#
# ## Why this is more than `ln -sf … .git/hooks/pre-push`
#
# `core.hooksPath` makes git ignore `.git/hooks` **entirely**. It is set
# globally on many machines — the git-lfs installer does it, as do most
# dotfile setups. When it is set, the old one-line symlink install put a
# hook in a directory git never looks at, and printed "Installed" anyway.
#
# That is not hypothetical: it is how the v0.2.7 … v0.2.10 release tags
# were pushed without the Linux/Windows validators in `scripts/pre-push`
# ever running. A silently-inert safety net is worse than none, because
# you stop checking.
#
# ## What this does instead
#
# Points `core.hooksPath` at a repo-local generated directory
# (`.githooks/`, gitignored) — a **--local** setting, so it overrides the
# global one for this clone only and touches no other repo.
#
# Chaining matters: repointing `core.hooksPath` would otherwise disable
# every hook the clone was inheriting. On this machine that includes a
# `commit-msg` hook that strips AI-attribution lines, plus the git-lfs
# family. So the generated hooks run this repo's `scripts/<hook>` first,
# then hand off to the inherited hook. Whatever worked before still works.
#
# Re-running is safe: the inherited path is recorded once in
# `claudepot.inheritedHooksPath` and reused, so a second run cannot
# chain `.githooks/` to itself.
set -euo pipefail

# --- --self-test ------------------------------------------------------
# Builds a throwaway repo whose inherited hooks behave like the ones on
# the machine this was written for, runs this script there, and drives
# the generated hooks. Two assertions, each a failure that shipped:
#
#   1. An inherited pre-push that delegates BACK into
#      "$root/.githooks/pre-push" must not loop. ~/.git-hooks/pre-push
#      does exactly that, assuming a repo using .githooks leaves
#      core.hooksPath alone — and this script does not, so on 2026-09-17
#      the two hooks called each other until 796 processes were running
#      and the release push hung. The fake below counts its own entries
#      and bails at five, so a regression FAILS instead of hanging.
#   2. A hook added to the inherited directory AFTER install must still
#      run. Shims used to be generated only for hooks present at install
#      time, so a later global pre-commit — a staged-secret check — was
#      silently never invoked in this clone.
if [ "${1:-}" = "--self-test" ]; then
  unset CLAUDEPOT_HOOK_PRE_PUSH CLAUDEPOT_HOOK_PRE_COMMIT
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  repo="$tmp/repo"; global="$tmp/global-hooks"
  export SELFTEST_LOG="$tmp/log"
  mkdir -p "$repo/scripts" "$global"
  git -C "$repo" init --quiet
  cp "$here" "$repo/scripts/install-hooks.sh"
  cat > "$repo/scripts/pre-push" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
echo validator >> "$SELFTEST_LOG"
EOF
  cat > "$global/pre-push" <<'EOF'
#!/usr/bin/env bash
payload="$(cat)"
n="$(grep -c '^global$' "$SELFTEST_LOG" 2>/dev/null || true)"
[ "${n:-0}" -lt 5 ] || { echo recursed >> "$SELFTEST_LOG"; exit 99; }
echo global >> "$SELFTEST_LOG"
root="$(git rev-parse --show-toplevel)"
if [ -x "$root/.githooks/pre-push" ]; then
  printf '%s\n' "$payload" | "$root/.githooks/pre-push" "$@" || exit $?
fi
EOF
  chmod +x "$repo/scripts/install-hooks.sh" "$repo/scripts/pre-push" "$global/pre-push"
  git -C "$repo" config --local core.hooksPath "$global"
  (cd "$repo" && ./scripts/install-hooks.sh >/dev/null)
  # Arrives only now, after install — like the global pre-commit did.
  cat > "$global/pre-commit" <<'EOF'
#!/usr/bin/env bash
echo late-pre-commit >> "$SELFTEST_LOG"
EOF
  chmod +x "$global/pre-commit"

  : > "$SELFTEST_LOG"
  rc=0
  (cd "$repo" && printf 'refs/heads/main 1 refs/heads/main 0\n' \
     | .githooks/pre-push origin selftest) || rc=$?
  if [ -x "$repo/.githooks/pre-commit" ]; then
    (cd "$repo" && .githooks/pre-commit) || true
  fi

  count() { grep -c "^$1\$" "$SELFTEST_LOG" 2>/dev/null || true; }
  problems=0
  check() { if [ "$2" = "$3" ]; then echo "  ok    $1"; else echo "  FAIL  $1 (got $2, want $3)"; problems=$((problems + 1)); fi; }
  echo "install-hooks self-test:"
  check "pre-push exits 0"                          "$rc" 0
  check "the inherited pre-push ran exactly once"   "$(count global)" 1
  check "no recursion"                              "$(count recursed)" 0
  check "scripts/pre-push ran exactly once"         "$(count validator)" 1
  check "a hook added after install still runs"     "$(count late-pre-commit)" 1
  if [ "$problems" -ne 0 ]; then
    echo "install-hooks self-test FAILED ($problems)"
    exit 1
  fi
  echo "install-hooks self-test ok"
  exit 0
fi

cd "$(git rev-parse --show-toplevel)"
repo_root="$(pwd)"
ours="$repo_root/.githooks"

# Absolute path, or empty if unset / nonexistent.
abspath() {
  [ -n "${1:-}" ] || return 0
  (cd "$1" 2>/dev/null && pwd) || true
}

current="$(abspath "$(git config --get core.hooksPath || true)")"
if [ "$current" = "$ours" ]; then
  # Re-run: recover the ORIGINAL inherited path, never our own dir.
  inherited="$(git config --get claudepot.inheritedHooksPath || true)"
else
  inherited="$current"
fi
[ -n "$inherited" ] || inherited="$repo_root/.git/hooks"

mkdir -p "$ours"
git config --local claudepot.inheritedHooksPath "$inherited"
git config --local core.hooksPath "$ours"

# --- pre-push: our release validators, then the inherited hook --------
# git feeds the ref list on stdin and BOTH consumers need to read it, so
# buffer once and replay. (git-lfs pre-push reads stdin too; without the
# replay whichever ran second would see EOF and silently do nothing.)
cat > "$ours/pre-push" <<'SHIM'
#!/usr/bin/env bash
# GENERATED by scripts/install-hooks.sh — do not edit.
set -euo pipefail
# Re-entry guard: the inherited pre-push may delegate back into this
# file, and this file chains to it — see the --self-test header.
[ -z "${CLAUDEPOT_HOOK_PRE_PUSH:-}" ] || exit 0
export CLAUDEPOT_HOOK_PRE_PUSH=1
root="$(git rev-parse --show-toplevel)"
inherited="$(git config --get claudepot.inheritedHooksPath || true)"

refs="$(cat)"

if [ -x "$root/scripts/pre-push" ]; then
  printf '%s\n' "$refs" | "$root/scripts/pre-push" "$@"
fi

if [ -n "$inherited" ] && [ -x "$inherited/pre-push" ]; then
  printf '%s\n' "$refs" | "$inherited/pre-push" "$@"
fi
SHIM
chmod +x "$ours/pre-push"

# --- passthrough shims for every other client hook ---------------------
# Generated for every hook name git invokes on a client, not for what the
# inherited directory holds today. A snapshot silently skips any hook
# added there later — a global pre-commit (a staged-secret check) never
# ran in this clone for that reason — and each shim already exits 0 when
# the inherited hook is absent, so a shim with nothing behind it costs
# one `git config` read. `reference-transaction` is left out on purpose:
# it fires on every ref update. `pre-push` is handled above.
CLIENT_HOOKS=(
  applypatch-msg pre-applypatch post-applypatch
  pre-commit pre-merge-commit prepare-commit-msg commit-msg post-commit
  pre-rebase post-checkout post-merge post-rewrite
  pre-auto-gc sendemail-validate
)
shimmed=()
for name in "${CLIENT_HOOKS[@]}"; do
  guard="CLAUDEPOT_HOOK_$(printf '%s' "$name" | tr 'a-z-' 'A-Z_')"
  cat > "$ours/$name" <<SHIM
#!/usr/bin/env bash
# GENERATED by scripts/install-hooks.sh — do not edit.
# Passthrough to the '$name' hook this clone inherited before
# core.hooksPath was pointed at .githooks/. Re-entry guarded, in case
# the inherited hook delegates back into .githooks/.
set -euo pipefail
[ -z "\${$guard:-}" ] || exit 0
export $guard=1
inherited="\$(git config --get claudepot.inheritedHooksPath || true)"
[ -n "\$inherited" ] && [ -x "\$inherited/$name" ] || exit 0
exec "\$inherited/$name" "\$@"
SHIM
  chmod +x "$ours/$name"
  if [ -x "$inherited/$name" ]; then shimmed+=("$name"); fi
done

# The old install's symlink is now dead weight and actively misleading —
# it looks installed while git ignores the whole directory.
if [ -L "$repo_root/.git/hooks/pre-push" ]; then
  rm -f "$repo_root/.git/hooks/pre-push"
  echo "Removed the stale .git/hooks/pre-push symlink (core.hooksPath supersedes it)."
fi

echo "core.hooksPath -> .githooks (local to this clone)"
echo "  pre-push      -> scripts/pre-push, then the inherited hook"
if [ ${#shimmed[@]} -gt 0 ]; then
  echo "  passthrough   -> ${shimmed[*]} (live now; the other client hooks are shimmed and idle)"
fi
echo "  inherited from: $inherited"

if [ ! -f "$repo_root/.validator-hosts" ]; then
  echo
  echo "Note: no .validator-hosts file found. Release-tag pushes will"
  echo "fail until you create one (see the header of scripts/pre-push;"
  echo "real host names live in CLAUDE.local.md)."
fi
