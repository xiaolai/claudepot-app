# Disk is full — 2026-05-02 14:32

**Status:** 92% used (74 GB free of 994 GB). The biggest single contributor is Xcode DerivedData at 38 GB.

## Top of Home

- ~/Library: 142 GB
- ~/Documents: 28 GB
- ~/Downloads: 21 GB
- ~/Pictures: 19 GB
- ~/Movies: 11 GB
- ~/Desktop: 4.3 GB
- ~/Music: 2.1 GB
- ~/Public: 14 KB

## Big files (>1 GB)

- 12 GB — ~/Movies/2024-italy-raw.mov
- 4.8 GB — ~/Downloads/Xcode_15.4.xip
- 3.2 GB — ~/Downloads/Anki-2.1.66.dmg
- 2.1 GB — ~/Documents/projects/old-archive.zip

## Safe-to-clear categories

**Xcode DerivedData (38 GB).** Build artifacts Xcode keeps for incremental compilation. Safe to clear; Xcode will rebuild what it needs on the next build. Command: `rm -rf ~/Library/Developer/Xcode/DerivedData/*`

**iOS Simulator devices (12 GB).** Each simulated iPhone/iPad takes 2–4 GB. If you don't currently need them: `xcrun simctl delete unavailable` (clears devices for OS versions you no longer have), then `xcrun simctl delete all` to wipe the rest.

**Sleep image (16 GB).** macOS uses this when the system enters deep sleep. Reclaimable but will regenerate. Don't bother unless you're under the wire.

**~/Downloads older than 30 days (4.2 GB across 67 files).** Review-then-delete; nothing here is required for the OS.

**~/.Trash (1.8 GB).** Empty when ready: `Cmd-Shift-Delete` in Finder, or `rm -rf ~/.Trash/*`.

## Recommendation

If you have 5 minutes:

1. **Empty Trash** (1.8 GB, zero risk).
2. **Clear Xcode DerivedData** (38 GB, low risk — Xcode rebuilds).
3. **Move the big Italy raw video off-disk** (12 GB, AirDrop to a NAS or external).

That's ~52 GB reclaimed without touching anything you'd miss.
