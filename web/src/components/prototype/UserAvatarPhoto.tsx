/**
 * Client child of UserAvatar — wraps the tier-1 <img> with an
 * onError handler that swaps to a boring-avatars identicon fallback.
 *
 * Why this is a client component
 * ------------------------------
 * The parent UserAvatar is a server component and stays that way:
 * tier 2 (agent sprite) and tier 3 (identicon) render entirely on
 * the server with zero client cost.
 *
 * Tier 1 (real photo) needs an event handler to react to load
 * failures (CSP block, 404, network), which forces a client
 * boundary. Boring-avatars renders here on the client *only* when
 * the photo fails — most pageloads never instantiate it because
 * the photo loads.
 *
 * History
 * -------
 * A previous version pre-rendered the fallback SVG on the server
 * via renderToStaticMarkup and passed it as a string prop. Next.js
 * 15's App Router rejects components that import react-dom/server
 * (incompatible with RSC serialization). The Vercel build failed
 * silently on 2026-05-18 — typecheck and unit tests passed, only
 * `next build` catches it. Lesson: web/ commits that touch the
 * RSC graph need a real build, not just tsc/test.
 *
 * Failure modes the fallback covers:
 *   - Vercel Blob 404 (avatar deleted out of band).
 *   - CSP block (belt-and-suspenders with tests/csp.test.ts).
 *   - Network timeout / offline.
 *
 * Visual: the fallback identicon is the same shape the user would
 * see if they had no photo, so degraded state is indistinguishable
 * from never-uploaded state.
 */

"use client";

import { useState } from "react";

import Avatar from "boring-avatars";

const PALETTE = ["#a35a2a", "#1a1a2e", "#374151", "#9ca3af", "#f5e6d8"];

type Variant = "beam" | "marble" | "pixel" | "ring" | "sunset" | "bauhaus";

interface Props {
  src: string;
  alt: string;
  username: string;
  size: number;
  className?: string;
  variant: Variant;
}

export function UserAvatarPhoto({
  src,
  alt,
  username,
  size,
  className,
  variant,
}: Props) {
  const [errored, setErrored] = useState(false);

  if (errored) {
    return (
      <span className={`proto-avatar ${className ?? ""}`} aria-label={alt}>
        <Avatar size={size} name={username} variant={variant} colors={PALETTE} />
      </span>
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
