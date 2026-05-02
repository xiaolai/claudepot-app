# Router admin port audit — 2026-05-02

**Status:** safe.

## LAN-side admin page

- http://192.168.1.1/ → 200 OK (Server: UniFi-OS, page title: "UniFi Network")
- https://192.168.1.1/ → 200 OK (same)

LAN-side admin is normal and expected.

## UPnP / external exposure

UPnP IGD reachable at http://192.168.1.1:1900/. Forwarded ports
declared:

- (none for router admin itself)
- 32400/tcp → 192.168.1.7 (Plex)
- 22/tcp → 192.168.1.5 (your NAS — was this intentional?)

The `22/tcp → NAS` rule is worth a second look. SSH from the
internet to your NAS is a meaningful exposure if you didn't
mean to enable it.
