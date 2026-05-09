/**
 * Render a media player for a submission's URL when the URL points
 * at one of the three supported platforms (YouTube, Spotify, Apple
 * Podcasts). Returns null otherwise.
 *
 * Wired into the post-detail page only (NOT the feed row) — a feed
 * of 30 posts each rendering an iframe wrecks scroll behavior. On
 * post detail, the auto-embed lands between the title and the body
 * so a reader who clicks into a podcast post sees the player
 * immediately, with the original link still visible above for
 * "open in new tab" / share.
 *
 * Source data is never mutated. The component derives the iframe
 * directly from `submission.url` at render time. Edits to the URL
 * (or future changes to embed src patterns) propagate without a
 * backfill.
 *
 * Privacy posture mirrors the in-body markdown embeds:
 *   - YouTube goes through youtube-nocookie.com.
 *   - Spotify and Apple don't have nocookie equivalents — the embeds
 *     do load each platform's session cookies if the reader is
 *     logged in there. We mitigate with sandbox + strict referrer
 *     policy. The same trade-off exists on every site embedding
 *     these platforms. See lib/markdown.ts for the parallel pattern
 *     applied to in-body URLs.
 */

import { extractApplePodcastsMatch } from "@/lib/apple-podcasts-embed";
import { extractSpotifyMatch } from "@/lib/spotify-embed";
import { extractYoutubeId } from "@/lib/youtube-embed";

interface Props {
  url: string | null | undefined;
}

export function UrlAutoEmbed({ url }: Props) {
  if (!url) return null;

  const youtubeId = extractYoutubeId(url);
  if (youtubeId) {
    return (
      <div className="proto-yt-embed">
        <iframe
          src={`https://www.youtube-nocookie.com/embed/${youtubeId}`}
          title="YouTube video"
          loading="lazy"
          referrerPolicy="strict-origin-when-cross-origin"
          sandbox="allow-scripts allow-same-origin allow-presentation"
          allow="accelerometer; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
          allowFullScreen
        />
      </div>
    );
  }

  const spotify = extractSpotifyMatch(url);
  if (spotify) {
    return (
      <div className="proto-spotify-embed">
        <iframe
          src={`https://open.spotify.com/embed/${spotify.kind}/${spotify.id}`}
          title="Spotify embed"
          loading="lazy"
          referrerPolicy="strict-origin-when-cross-origin"
          sandbox="allow-scripts allow-same-origin allow-popups"
          allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture"
        />
      </div>
    );
  }

  const apple = extractApplePodcastsMatch(url);
  if (apple) {
    const base = `https://embed.podcasts.apple.com/${apple.country}/podcast/${apple.slug}/id${apple.showId}`;
    const src = apple.episodeId ? `${base}?i=${apple.episodeId}` : base;
    return (
      <div className="proto-applepod-embed">
        <iframe
          src={src}
          title="Apple Podcasts embed"
          loading="lazy"
          referrerPolicy="strict-origin-when-cross-origin"
          sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
          allow="autoplay; encrypted-media; fullscreen"
        />
      </div>
    );
  }

  return null;
}
