/**
 * Avatar — three-tier rendering:
 *   1. imageUrl present (real OAuth photo or user upload from
 *      setAvatar) → <img> via UserAvatarPhoto. The wrapping client
 *      component swaps to a boring-avatars identicon if the photo
 *      fails to load (CSP block, 404, network), so a broken-image
 *      glyph never reaches the user.
 *   2. username matches an agent pattern (e.g. "mcp-tool-watch") → 16×16
 *      hand-drawn pixel sprite via renderAgentSprite. 12 archetype designs
 *      × 9 variant palettes = 108 unique sprites.
 *   3. fallback → boring-avatars beam identicon (humans, fixture users,
 *      anything not matching tiers 1 or 2).
 *
 * Server component for tiers 2 and 3 (inline SVG, zero round-trips).
 * Tier 1 hands off to UserAvatarPhoto for the onError handler.
 *
 * Past mistake: a previous version pre-rendered the tier-1 fallback
 * SVG on the server via renderToStaticMarkup. Next.js 15's App Router
 * rejects components that pull in react-dom/server (RSC-incompatible)
 * — the Vercel build failed silently while tsc/test passed. The
 * fallback now lives entirely in the client child component.
 */

import Avatar from "boring-avatars";

import { parseAgentUsername, renderAgentSprite } from "@/lib/agent-sprites";

import { UserAvatarPhoto } from "./UserAvatarPhoto";

const PALETTE = ["#a35a2a", "#1a1a2e", "#374151", "#9ca3af", "#f5e6d8"];

type Variant = "beam" | "marble" | "pixel" | "ring" | "sunset" | "bauhaus";

interface Props {
  username: string;
  imageUrl?: string | null;
  size?: number;
  className?: string;
  /** Override identicon variant. Ignored when imageUrl or agent sprite applies. */
  variant?: Variant;
}

export function UserAvatar({
  username,
  imageUrl,
  size = 32,
  className,
  variant = "beam",
}: Props) {
  // Tier 1 — real photo, with onError fallback to identicon (rendered
  // client-side by UserAvatarPhoto).
  if (imageUrl) {
    return (
      <UserAvatarPhoto
        src={imageUrl}
        alt={`@${username}`}
        username={username}
        size={size}
        className={className}
        variant={variant}
      />
    );
  }

  // Tier 2 — agent sprite (if username parses as one of the 108).
  const agent = parseAgentUsername(username);
  if (agent) {
    const svg = renderAgentSprite(agent, size);
    return (
      <span
        className={`proto-avatar ${className ?? ""}`}
        aria-label={`@${username} (agent)`}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    );
  }

  // Tier 3 — identicon for humans.
  return (
    <span className={`proto-avatar ${className ?? ""}`} aria-label={`@${username}`}>
      <Avatar size={size} name={username} variant={variant} colors={PALETTE} />
    </span>
  );
}
