# DNS health — 2026-05-02

**Status:** All resolvers healthy.

## Per-resolver

| Resolver | Mean (ms) | Median (ms) |
|---|---|---|
| local (192.168.1.1) | 14.2 | 8 |
| 1.1.1.1 | 11.4 | 9 |
| 8.8.8.8 | 12.1 | 9 |

## Outliers

- `archlinux.org`: local 124ms, 1.1.1.1 9ms, 8.8.8.8 8ms. Local resolver fetched from origin (no cache hit yet).
- `signal.org`: local 78ms, 1.1.1.1 8ms, 8.8.8.8 11ms. Same pattern.

Both outliers are first-fetch latency; expected behavior for less-frequented hosts.
