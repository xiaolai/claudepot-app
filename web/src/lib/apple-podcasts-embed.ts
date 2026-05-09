/**
 * Apple Podcasts embed pre-pass for the markdown renderer.
 *
 * Mirrors the YouTube + Spotify pre-pass shape: a bare-URL paragraph
 * pointing at podcasts.apple.com is rewritten to a raw-HTML iframe
 * BEFORE marked.parse so the rewritten block lands as raw HTML in the
 * marked input.
 *
 * The transformation is mechanically simple: swap the host
 * `podcasts.apple.com` → `embed.podcasts.apple.com` and preserve the
 * rest of the path (including the optional `?i=<episode-id>` query
 * for episode-level embeds). Apple's embed surface is canonical and
 * documented; the URL shape on either side of the swap is
 *
 *   https://podcasts.apple.com/{country}/podcast/{slug}/id{numeric}[?i={numeric}]
 *
 * Defense in depth: every emitted iframe has src locked to
 * `https://embed.podcasts.apple.com/...`. The sanitize-html step in
 * markdown.ts re-validates the src via APPLE_PODCASTS_EMBED_SRC.
 *
 * Privacy note: Apple does not provide a nocookie-equivalent host.
 * The embed loads embed.podcasts.apple.com, where the reader may
 * have a session cookie. By dropping `allow-same-origin` from the
 * sandbox (see lib/embed-attrs.ts) the iframe is forced into a
 * unique opaque origin and cannot access those cookies. Same
 * trade-off and mitigation as Spotify.
 */

import { APPLE_PODCASTS_IFRAME_ATTRS } from "@/lib/embed-attrs";

const COUNTRY = /^[a-z]{2}$/;
const NUMERIC_ID = /^\d+$/;

export interface ApplePodcastsMatch {
  /** ISO-3166 alpha-2 country code, lowercased. */
  country: string;
  /** URL-safe slug from the canonical podcasts.apple.com URL. */
  slug: string;
  /** Numeric show id (the "id<n>" segment in the URL). */
  showId: string;
  /** Numeric episode id (the `?i=<n>` query param), or null for show-level. */
  episodeId: string | null;
}

/**
 * Extract an Apple Podcasts match from a URL. Returns null if the
 * URL isn't a recognised shape.
 *
 * Recognised shapes:
 *   - https://podcasts.apple.com/{country}/podcast/{slug}/id{num}
 *   - https://podcasts.apple.com/{country}/podcast/{slug}/id{num}?i={num}
 *
 * The slug is opaque to us (Apple regenerates it from the show
 * title); we pass it through verbatim. The numeric IDs are validated
 * to defend against path-traversal-shaped values landing in the
 * embed src.
 */
export function extractApplePodcastsMatch(
  url: string,
): ApplePodcastsMatch | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const host = parsed.hostname.replace(/^www\./, "");
  if (host !== "podcasts.apple.com") return null;

  // Path: /{country}/podcast/{slug}/id{numeric}
  const m = parsed.pathname.match(
    /^\/([a-z]{2})\/podcast\/([^/]+)\/id(\d+)\/?$/,
  );
  if (!m) return null;
  const [, country, slug, showId] = m;
  if (!COUNTRY.test(country) || !NUMERIC_ID.test(showId)) return null;

  // ?i=<episode-id> is the only query param we honor. If it's
  // present but not numeric, the user signaled episode intent with a
  // malformed value — reject the whole URL rather than silently
  // downgrading to a show-level embed.
  if (parsed.searchParams.has("i")) {
    const episodeRaw = parsed.searchParams.get("i") ?? "";
    if (!NUMERIC_ID.test(episodeRaw)) return null;
    return { country, slug, showId, episodeId: episodeRaw };
  }

  return { country, slug, showId, episodeId: null };
}

/** Build the iframe block for a given match. Iframe attrs come from
 *  lib/embed-attrs.ts so the in-body and post-detail surfaces share
 *  one source of truth. Apple's official embed sandbox is famously
 *  permissive; we deliberately tighten in the shared module — if a
 *  show breaks playback, loosen there, not here. */
function buildEmbed(match: ApplePodcastsMatch): string {
  const base = `https://embed.podcasts.apple.com/${match.country}/podcast/${match.slug}/id${match.showId}`;
  const src = match.episodeId ? `${base}?i=${match.episodeId}` : base;
  const a = APPLE_PODCASTS_IFRAME_ATTRS;
  return (
    `<div class="proto-applepod-embed">` +
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
 * Pre-process a markdown source string, rewriting bare Apple Podcasts
 * paragraph URLs into raw-HTML iframe blocks. Same fence-aware walker
 * as the YouTube and Spotify pre-passes.
 */
export function rewriteApplePodcastsEmbeds(source: string): string {
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
      const match = extractApplePodcastsMatch(urlMatch[1]);
      if (match) {
        out.push(buildEmbed(match));
        continue;
      }
    }

    out.push(line);
  }

  return out.join("\n");
}
