# Bandwidth-hog inference — 2026-05-02

**Status:** Backblaze backing up — accounts for ~80% of upload.

## Top 10 processes (by total bytes)

| Process | Bytes in | Bytes out | Notes |
|---|---|---|---|
| Backblaze (bzfilelist) | 0.1 MB | 312 MB | Daily backup window. |
| Chrome | 28 MB | 4.1 MB | Background tabs (one is YouTube auto-play). |
| Slack | 12 MB | 2.0 MB | Idle workspace. |
| Brave | 6 MB | 0.8 MB | Reader-tab activity. |
| iCloud | 2 MB | 5 MB | Photo sync. |
| Spotlight | 1 MB | 0.0 MB | Metadata refresh. |

If you don't recognize the top entry, check Activity Monitor → Network.
