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
 * (the way YouTube does). The embed loads open.spotify.com which can
 * read Spotify's session cookies. We mitigate with sandbox +
 * referrerpolicy on the iframe; user cookies still flow to Spotify
 * if the reader is logged in there. This is the same trade-off any
 * site embedding Spotify makes.
 */

/** Spotify content ID — 22-char base62 (alphanumeric).
 *  Officially documented as "alphanumeric"; in practice base62.
 *  Lock to that to avoid path-traversal-shaped values. */
const SPOTIFY_ID = /^[a-zA-Z0-9]{22}$/;

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

  const m = path.match(/^\/(episode|show)\/([a-zA-Z0-9]+)\/?/);
  if (!m) return null;
  const id = m[2];
  if (!SPOTIFY_ID.test(id)) return null;
  return { kind: m[1] as SpotifyKind, id };
}

/** Build the iframe block for a given match. */
function buildEmbed(match: SpotifyMatch): string {
  const src = `https://open.spotify.com/embed/${match.kind}/${match.id}`;
  // The wrapper carries the fixed height in CSS (.proto-spotify-embed)
  // so the iframe itself can be width/height 100% and adapt to the
  // content column.
  return (
    `<div class="proto-spotify-embed">` +
    `<iframe src="${src}"` +
    ` title="Spotify embed"` +
    ` loading="lazy"` +
    ` referrerpolicy="strict-origin-when-cross-origin"` +
    ` sandbox="allow-scripts allow-same-origin allow-popups"` +
    ` allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture"` +
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
    const fenceMatch = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (fenceMatch) {
      const marker = fenceMatch[1];
      const ch = marker[0] as "`" | "~";
      if (!inFence) {
        inFence = true;
        fenceChar = ch;
        fenceLen = marker.length;
        out.push(line);
        continue;
      }
      if (ch === fenceChar && marker.length >= fenceLen) {
        inFence = false;
        fenceChar = null;
        fenceLen = 0;
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
