/**
 * Avatar — three-tier rendering:
 *   1. imageUrl present (real OAuth photo or user upload from
 *      setAvatar) → <img> via UserAvatarPhoto. The wrapping client
 *      component swaps to a pre-rendered fallback SVG if the photo
 *      fails to load (CSP block, 404, network), so a broken-image
 *      glyph never reaches the user.
 *   2. username matches an agent pattern (e.g. "mcp-tool-watch") → 16×16
 *      hand-drawn pixel sprite via renderAgentSprite. 12 archetype designs
 *      × 9 variant palettes = 108 unique sprites.
 *   3. fallback → boring-avatars beam identicon (humans, fixture users,
 *      anything not matching tiers 1 or 2).
 *
 * The fallback for tier 1 is chosen by the same parse-then-identicon
 * cascade as tiers 2 and 3, so a user whose photo fails reverts to
 * exactly what they'd see if they'd never uploaded — agents fall back
 * to their sprite, humans to their identicon.
 *
 * Server component for tiers 2 and 3 (inline SVG, zero round-trips).
 * Tier 1 hands off to a tiny client child for the onError handler;
 * boring-avatars stays out of the client bundle because the fallback
 * SVG is pre-rendered to a string on the server.
 */

import Avatar from "boring-avatars";
import { renderToStaticMarkup } from "react-dom/server";

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

function renderIdenticonSvg(
  username: string,
  size: number,
  variant: Variant,
): string {
  return renderToStaticMarkup(
    <Avatar size={size} name={username} variant={variant} colors={PALETTE} />,
  );
}

export function UserAvatar({
  username,
  imageUrl,
  size = 32,
  className,
  variant = "beam",
}: Props) {
  const agent = parseAgentUsername(username);

  // Tier 1 — real photo, with onError fallback to tier 2 or tier 3.
  if (imageUrl) {
    const fallbackSvg = agent
      ? renderAgentSprite(agent, size)
      : renderIdenticonSvg(username, size, variant);
    return (
      <UserAvatarPhoto
        src={imageUrl}
        alt={`@${username}`}
        size={size}
        className={className}
        fallbackSvg={fallbackSvg}
      />
    );
  }

  // Tier 2 — agent sprite (if username parses as one of the 108).
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
