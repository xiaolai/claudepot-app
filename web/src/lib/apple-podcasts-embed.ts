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
 * The embed loads embed.podcasts.apple.com which can read Apple's
 * session cookies. Same trade-off as Spotify — mitigated by sandbox +
 * referrerpolicy, but cookies still flow if the reader is logged
 * into Apple in the same browser.
 */

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

  // ?i=<episode-id> is the only query param we honor.
  const episodeRaw = parsed.searchParams.get("i");
  const episodeId =
    episodeRaw && NUMERIC_ID.test(episodeRaw) ? episodeRaw : null;

  return { country, slug, showId, episodeId };
}

/** Build the iframe block for a given match. */
function buildEmbed(match: ApplePodcastsMatch): string {
  const base = `https://embed.podcasts.apple.com/${match.country}/podcast/${match.slug}/id${match.showId}`;
  const src = match.episodeId ? `${base}?i=${match.episodeId}` : base;
  // Apple's official embed sandbox is famously permissive
  // (allow-storage-access-by-user-activation, allow-top-navigation,
  // etc.). We deliberately tighten — readers don't need the embed
  // to navigate the host page or escape its sandbox. If the player
  // breaks for some content shape, expand on a case-by-case basis.
  return (
    `<div class="proto-applepod-embed">` +
    `<iframe src="${src}"` +
    ` title="Apple Podcasts embed"` +
    ` loading="lazy"` +
    ` referrerpolicy="strict-origin-when-cross-origin"` +
    ` sandbox="allow-scripts allow-same-origin allow-popups allow-forms"` +
    ` allow="autoplay; encrypted-media; fullscreen"` +
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
