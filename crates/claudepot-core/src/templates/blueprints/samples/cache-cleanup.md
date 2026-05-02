# Cache cleanup audit — 2026-05-02

**Status:** total reclaimable: 47 GB.

## What's where

| Path | Size | What it is | Safe to clear? | Command |
|---|---|---|---|---|
| ~/Library/Developer/Xcode/DerivedData | 38 GB | Xcode incremental-build artifacts | ✓ | `rm -rf ~/Library/Developer/Xcode/DerivedData/*` |
| ~/Library/Developer/CoreSimulator/Devices | 8.4 GB | iOS Simulator devices | ✓ | `xcrun simctl delete unavailable` first, then `xcrun simctl delete all` to wipe |
| ~/Library/Caches | 1.9 GB | Per-app caches (mail, browsers, etc.) | ⚠ | `rm -rf ~/Library/Caches/*` (browsers will rebuild; you stay logged in) |
| ~/.npm | 720 MB | npm package cache | ✓ | `npm cache clean --force` |
| ~/Library/pnpm/store | 480 MB | pnpm content-addressed store | ⚠ | `pnpm store prune` (kept-but-unused packages) |
| ~/Library/Caches/Homebrew | 320 MB | Downloaded formulae bottles | ✓ | `brew cleanup --prune=all` |
| ~/.Trash | 280 MB | Trash | ✓ | empty via Finder, or `rm -rf ~/.Trash/*` |
| ~/Library/Caches/Google/Chrome | 240 MB | Chrome cache | ⚠ | clear via Chrome Settings; stays logged in |
| ~/Library/Caches/com.apple.Safari | 110 MB | Safari cache | ⚠ | Safari → Develop → Empty Cache |

## Recommendation

If you have 5 minutes:

1. **Empty Trash** (280 MB, zero risk).
2. **Clear Xcode DerivedData** (38 GB, no risk — Xcode rebuilds).
3. **Run `brew cleanup --prune=all`** (320 MB).

That's ~38.6 GB reclaimed without touching anything you'll miss.
