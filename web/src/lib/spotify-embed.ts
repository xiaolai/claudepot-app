/**
 * Spotify embed pre-pass for the markdown renderer.
 *
 * Mirrors the YouTube pre-pass shape: a bare-URL paragraph is
 * rewritten to a raw-HTML iframe BEFORE marked.parse so the rewritten
 * block lands as raw HTML in the marked input. Only one trigger
 * shape today — bare URL on its own line — because Spotify URLs are
 * long enough that an inline directive would be awkward, and the
 * primary surface (post bodies, not comments) makes own-line
 * paste the dominant pattern.
 *
 * Defense in depth: every emitted iframe has src locked to
 * `https://open.spotify.com/embed/{episode|show}/<22-char-id>`. The
 * sanitize-html step in markdown.ts re-validates the src via
 * SPOTIFY_EMBED_SRC; if the regex below ever drifts the sanitizer
 * drops the embed entirely rather than emit a bad iframe.
 *
 * Privacy note: Spotify does not provide a nocookie-equivalent host
 * (the way YouTube does). The embed loads open.spotify.com, where
 * the reader may have a session cookie. By dropping `allow-same-
 * origin` from the sandbox (see lib/embed-attrs.ts) the iframe is
 * forced into a unique opaque origin and cannot access those
 * cookies. The player communicates with the parent via postMessage,
 * which is cross-origin-safe.
 */

import { SPOTIFY_IFRAME_ATTRS } from "@/lib/embed-attrs";

export type SpotifyKind = "episode" | "show";

export interface SpotifyMatch {
  kind: SpotifyKind;
  id: string;
}

/**
 * Extract a Spotify episode or show match from a URL string. Returns
 * null if the URL doesn't match a recognised shape or the ID isn't
 * the canonical 22-character form.
 *
 * Recognised shapes:
 *   - https://open.spotify.com/episode/<22-char-id>
 *   - https://open.spotify.com/show/<22-char-id>
 *   - Locale prefix tolerated: open.spotify.com/intl-<locale>/episode/<id>
 *   - Trailing `?si=…` and other query params are dropped — most are
 *     user-tracking surface we don't want carrying through to readers.
 *
 * Tracks (`/track/<id>`), albums (`/album/<id>`), and playlists are
 * intentionally not matched. The polity's editorial scope is podcasts.
 */
export function extractSpotifyMatch(url: string): SpotifyMatch | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const host = parsed.hostname.replace(/^www\./, "");
  if (host !== "open.spotify.com") return null;

  // Strip optional locale prefix (`/intl-<locale>/`) before matching.
  const path = parsed.pathname.replace(/^\/intl-[a-z-]+\//, "/");

  // End-anchored: `/episode/<id>` or `/show/<id>` with optional
  // trailing slash, nothing after. Rejects `/episode/<id>/extra`
  // shapes that the older non-anchored pattern silently coerced into
  // an embed.
  const m = path.match(/^\/(episode|show)\/([A-Za-z0-9]{22})\/?$/);
  if (!m) return null;
  return { kind: m[1] as SpotifyKind, id: m[2] };
}

/** Build the iframe block for a given match. Iframe attrs come from
 *  lib/embed-attrs.ts so the in-body and post-detail surfaces share
 *  one source of truth — drift between the two is the failure mode
 *  the centralization is preventing. The wrapper carries the fixed
 *  height in CSS (.proto-spotify-embed). */
function buildEmbed(match: SpotifyMatch): string {
  const src = `https://open.spotify.com/embed/${match.kind}/${match.id}`;
  const a = SPOTIFY_IFRAME_ATTRS;
  return (
    `<div class="proto-spotify-embed">` +
    `<iframe src="${src}"` +
    ` title="${a.title}"` +
    ` loading="${a.loading}"` +
    ` referrerpolicy="${a.referrerpolicy}"` +
    ` sandbox="${a.sandbox}"` +
    ` allow="${a.allow}"` +
    `></iframe>` +
    `</div>`
  );
}

/**
 * Pre-process a markdown source string, rewriting bare Spotify
 * paragraph URLs into raw-HTML iframe blocks. Idempotent: emitted
 * blocks don't re-match the trigger.
 *
 * Walks the source line by line and skips lines inside fenced code
 * blocks (``` or ~~~) so a Spotify URL inside a code sample is
 * preserved verbatim. Indented code blocks (4+ space indent) are
 * skipped via the 0-3-space leading-indent gate on the trigger
 * regex — same threshold CommonMark uses.
 */
export function rewriteSpotifyEmbeds(source: string): string {
  const lines = source.split("\n");
  const out: string[] = [];
  let inFence = false;
  let fenceChar: "`" | "~" | null = null;
  let fenceLen = 0;

  const URL_LINE = /^ {0,3}(https?:\/\/\S+)[ \t]*$/;

  for (const line of lines) {
    // Fence open/close detection — see youtube-embed.ts for the
    // CommonMark rules. Strict close (no info string) prevents
    // a line like `~~~more` inside a fence body from closing it.
    if (inFence) {
      const closeMatch = line.match(/^ {0,3}(`{3,}|~{3,})[ \t]*$/);
      if (closeMatch) {
        const marker = closeMatch[1];
        const ch = marker[0] as "`" | "~";
        if (ch === fenceChar && marker.length >= fenceLen) {
          inFence = false;
          fenceChar = null;
          fenceLen = 0;
          out.push(line);
          continue;
        }
      }
    } else {
      const openMatch = line.match(/^ {0,3}(`{3,}|~{3,})/);
      if (openMatch) {
        const marker = openMatch[1];
        const ch = marker[0] as "`" | "~";
        inFence = true;
        fenceChar = ch;
        fenceLen = marker.length;
        out.push(line);
        continue;
      }
    }

    if (inFence) {
      out.push(line);
      continue;
    }

    const urlMatch = line.match(URL_LINE);
    if (urlMatch) {
      const match = extractSpotifyMatch(urlMatch[1]);
      if (match) {
        out.push(buildEmbed(match));
        continue;
      }
    }

    out.push(line);
  }

  return out.join("\n");
}
