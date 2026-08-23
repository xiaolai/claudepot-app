#!/usr/bin/env bash
# Rebuild the remote panel into the directory `remote::assets` embeds.
#
# The output is committed, so this is the step that makes a source change
# in `panel/` real. Running it is idempotent; not running it ships the
# previous bundle with no error anywhere.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/crates/claudepot-core/src/remote/assets/panel"

cd "$repo/panel"

if [ ! -d node_modules ]; then
  echo "==> installing panel dependencies"
  pnpm install
fi

echo "==> building panel"
pnpm build

# The Rust side `include_bytes!`es these by literal name. A build that
# silently produced nothing would leave the previous bundle in place and
# look like success — assert instead of trusting the exit code.
for f in index.html panel.js panel.css \
         fonts/InstrumentSans-latin.woff2 fonts/InstrumentSans-latin-ext.woff2 \
         fonts/InstrumentSerif-latin.woff2 fonts/InstrumentSerif-latin-ext.woff2 \
         fonts/JetBrainsMono-latin.woff2 fonts/JetBrainsMono-latin-ext.woff2; do
  if [ ! -s "$out/$f" ]; then
    echo "error: $out/$f is missing or empty after the build" >&2
    exit 1
  fi
done

# ── The chunk route table ────────────────────────────────────────────
#
# Mermaid splits itself into ~60 lazily-imported diagram packs, and
# `remote::assets` resolves request paths with an exhaustive match over
# literals — there is no filesystem lookup to fall back on, which is the
# property that gives that module nothing to defend against traversal.
#
# So the arms are generated from whatever the build actually emitted.
# Hand-maintaining sixty of them would be wrong within one mermaid
# upgrade, and a directory walk at runtime would trade the property away.
# A test in `remote::assets` walks this directory and fails if the table
# and the files disagree in either direction.
gen="$repo/crates/claudepot-core/src/remote/assets/panel_chunks.rs"
echo "==> generating $(basename "$gen")"
{
  echo "//! Lazily-loaded panel chunks — **generated**, do not edit."
  echo "//!"
  echo "//! Written by \`scripts/build-panel.sh\` from the files Vite emitted."
  echo "//! Mermaid alone splits into dozens of diagram packs; listing them by"
  echo "//! hand would be wrong within one upgrade of it."
  echo "//!"
  echo "//! \`remote::assets::tests::every_chunk_on_disk_is_reachable\` walks the"
  echo "//! directory and fails in both directions, so a rebuild that was never"
  echo "//! committed and an arm whose file vanished are both caught."
  echo ""
  echo "/// Resolve a bare chunk filename. \`None\` for anything not emitted."
  echo "pub(super) fn chunk(name: &str) -> Option<&'static [u8]> {"
  echo "    Some(match name {"
  for f in "$out"/chunks/*.js; do
    b="$(basename "$f")"
    printf '        "%s" => include_bytes!("panel/chunks/%s"),\n' "$b" "$b"
  done
  echo "        _ => return None,"
  echo "    })"
  echo "}"
} > "$gen"
# Format what we just generated, or the next `cargo fmt --check` fails
# on it. The naive emit puts every arm on one line and rustfmt wants the
# long ones wrapped, so without this the tree is left RED by a build step
# the README tells people to run — and the diff shows up in whatever
# commit happens next, looking like an unrelated change.
if command -v rustfmt >/dev/null 2>&1; then
  rustfmt --edition 2021 "$gen"
else
  # Loud, not silent: leaving an unformatted generated file behind is a
  # broken gate, and it must not look like success.
  echo "warning: rustfmt not found — run 'cargo fmt' before committing," >&2
  echo "         or CI's Format job will fail on $(basename "$gen")" >&2
fi

chunks=$(ls "$out"/chunks/*.js 2>/dev/null | wc -l | tr -d ' ')
echo "==> $chunks chunk(s) routed"

echo "==> ok: $out"
