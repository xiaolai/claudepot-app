/**
 * Single source of truth for the per-platform iframe attribute sets
 * used by media embeds (YouTube, Spotify, Apple Podcasts).
 *
 * Two consumers must agree on these:
 *   1. The markdown sanitizer in lib/markdown.ts re-stamps each
 *      surviving iframe's attrs to the matching set, so a hand-rolled
 *      iframe can't sneak in a relaxed sandbox.
 *   2. The UrlAutoEmbed component on the post-detail page emits the
 *      same iframe directly from submission.url, never going through
 *      the markdown pipeline.
 *
 * Centralizing here lets a tightening (or platform-specific quirk
 * fix) land in one edit and propagate everywhere. Drift between the
 * two consumers is the failure mode the centralization is preventing.
 *
 * Sandbox posture: minimal permissions. allow-same-origin is
 * deliberately NOT granted to Spotify or Apple — those embeds run
 * on cookie-bearing hosts (open.spotify.com, embed.podcasts.apple.com)
 * where the reader may be logged in. Dropping allow-same-origin
 * forces the iframe into a unique opaque origin so it can't read
 * those cookies. The embed players communicate with the parent via
 * postMessage, which works cross-origin without same-origin. YouTube
 * gets allow-same-origin because youtube-nocookie.com is the
 * canonical privacy-aware embed host (no Google-account cookies live
 * there), so the same-origin combo doesn't grant access to anything
 * sensitive — and removing it breaks YouTube's embed playback
 * controls.
 */

export const YT_IFRAME_ATTRS = {
  title: "YouTube video",
  loading: "lazy",
  referrerpolicy: "strict-origin-when-cross-origin",
  sandbox: "allow-scripts allow-same-origin allow-presentation",
  allow:
    "accelerometer; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share",
  allowfullscreen: "",
} as const;

export const SPOTIFY_IFRAME_ATTRS = {
  title: "Spotify embed",
  loading: "lazy",
  referrerpolicy: "strict-origin-when-cross-origin",
  // No allow-same-origin: see file header. allow-popups is kept for
  // the "Open in Spotify" button to work in a new tab.
  sandbox: "allow-scripts allow-popups",
  allow:
    "autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture",
} as const;

export const APPLE_PODCASTS_IFRAME_ATTRS = {
  title: "Apple Podcasts embed",
  loading: "lazy",
  referrerpolicy: "strict-origin-when-cross-origin",
  // No allow-same-origin: see file header. allow-popups for "Open in
  // Apple Podcasts" link; allow-forms for the player's internal
  // controls (Apple's official sandbox grants both).
  sandbox: "allow-scripts allow-popups allow-forms",
  allow: "autoplay; encrypted-media; fullscreen",
} as const;
