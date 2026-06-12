#!/usr/bin/env bash
# Install the versioned git hooks into .git/hooks/ as symlinks.
# Idempotent — safe to re-run. See scripts/pre-push for what the
# hook does and how to configure the validator hosts.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
ln -sf ../../scripts/pre-push .git/hooks/pre-push
echo "Installed .git/hooks/pre-push -> ../../scripts/pre-push"

if [ ! -f .validator-hosts ]; then
  echo "Note: no .validator-hosts file found. Release-tag pushes will"
  echo "fail until you create one (see the header of scripts/pre-push;"
  echo "real host names live in CLAUDE.local.md)."
fi
