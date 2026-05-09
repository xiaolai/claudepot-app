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
 * directly from `submission.url` at render time.
 *
 * Iframe attribute sets are imported from lib/embed-attrs.ts — the
 * same source the markdown sanitizer uses, so the in-body and
 * post-detail surfaces can't drift on sandbox/referrer/allow.
 *
 * Privacy posture mirrors the in-body markdown embeds:
 *   - YouTube goes through youtube-nocookie.com.
 *   - Spotify and Apple don't have nocookie equivalents — the embeds
 *     do load each platform's pages, but with allow-same-origin
 *     dropped from the sandbox the iframe can't read those hosts'
 *     session cookies. See lib/embed-attrs.ts for the trade-off.
 */

import { extractApplePodcastsMatch } from "@/lib/apple-podcasts-embed";
import {
  APPLE_PODCASTS_IFRAME_ATTRS,
  SPOTIFY_IFRAME_ATTRS,
  YT_IFRAME_ATTRS,
} from "@/lib/embed-attrs";
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
          title={YT_IFRAME_ATTRS.title}
          loading={YT_IFRAME_ATTRS.loading}
          referrerPolicy={YT_IFRAME_ATTRS.referrerpolicy}
          sandbox={YT_IFRAME_ATTRS.sandbox}
          allow={YT_IFRAME_ATTRS.allow}
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
          title={SPOTIFY_IFRAME_ATTRS.title}
          loading={SPOTIFY_IFRAME_ATTRS.loading}
          referrerPolicy={SPOTIFY_IFRAME_ATTRS.referrerpolicy}
          sandbox={SPOTIFY_IFRAME_ATTRS.sandbox}
          allow={SPOTIFY_IFRAME_ATTRS.allow}
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
          title={APPLE_PODCASTS_IFRAME_ATTRS.title}
          loading={APPLE_PODCASTS_IFRAME_ATTRS.loading}
          referrerPolicy={APPLE_PODCASTS_IFRAME_ATTRS.referrerpolicy}
          sandbox={APPLE_PODCASTS_IFRAME_ATTRS.sandbox}
          allow={APPLE_PODCASTS_IFRAME_ATTRS.allow}
        />
      </div>
    );
  }

  return null;
}
