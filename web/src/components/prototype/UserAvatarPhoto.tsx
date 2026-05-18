/**
 * Client child of UserAvatar — wraps the tier-1 <img> with an
 * onError handler that swaps to a pre-rendered fallback SVG.
 *
 * Why this exists
 * ---------------
 * The parent UserAvatar is a server component and stays that way:
 * tiers 2 and 3 render boring-avatars / agent sprites entirely on
 * the server, keeping ~3KB of identicon code out of the client
 * bundle.
 *
 * Tier 1 (real photo) needs an event handler to react to load
 * failures (CSP block, 404, network), which forces a client
 * boundary. We isolate that boundary to this file so the heavier
 * SVG generators never get pulled into the client graph — the
 * parent pre-renders the fallback as an SVG string and we just
 * dangerouslySetInnerHTML it on error.
 *
 * Failure modes the fallback covers:
 *   - Vercel Blob 404 (avatar deleted out of band, or filename drift).
 *   - CSP block (a future image host added to setAvatar without
 *     a matching middleware allowlist — paired with tests/csp.test.ts
 *     this is belt-and-suspenders).
 *   - Network timeout / offline.
 *
 * The fallback SVG is the same identicon the user would see if
 * they had no photo at all (or the matching agent sprite for the
 * 108 agent usernames), so degraded state is visually identical
 * to never-uploaded state.
 */

"use client";

import { useState } from "react";

interface Props {
  src: string;
  alt: string;
  size: number;
  className?: string;
  fallbackSvg: string;
}

export function UserAvatarPhoto({
  src,
  alt,
  size,
  className,
  fallbackSvg,
}: Props) {
  const [errored, setErrored] = useState(false);

  if (errored) {
    return (
      <span
        className={`proto-avatar ${className ?? ""}`}
        aria-label={alt}
        dangerouslySetInnerHTML={{ __html: fallbackSvg }}
      />
    );
  }

  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={src}
      alt={alt}
      width={size}
      height={size}
      className={`proto-avatar ${className ?? ""}`}
      onError={() => setErrored(true)}
    />
  );
}
