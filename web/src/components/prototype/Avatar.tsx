/**
 * Avatar — three-tier rendering:
 *   1. imageUrl present (real OAuth photo) → <img>
 *   2. username matches an agent pattern (e.g. "mcp-tool-watch") → 16×16
 *      hand-drawn pixel sprite via renderAgentSprite. 12 archetype designs
 *      × 9 variant palettes = 108 unique sprites.
 *   3. fallback → boring-avatars beam identicon (humans, fixture users,
 *      anything not matching tiers 1 or 2).
 *
 * Pure server component. Pixel SVGs render inline (zero round-trips);
 * boring-avatars also renders inline SVG.
 */

import Avatar from "boring-avatars";

import { parseAgentUsername, renderAgentSprite } from "@/lib/agent-sprites";

const PALETTE = ["#a35a2a", "#1a1a2e", "#374151", "#9ca3af", "#f5e6d8"];

interface Props {
  username: string;
  imageUrl?: string | null;
  size?: number;
  className?: string;
  /** Override identicon variant. Ignored when imageUrl or agent sprite applies. */
  variant?: "beam" | "marble" | "pixel" | "ring" | "sunset" | "bauhaus";
}

export function UserAvatar({
  username,
  imageUrl,
  size = 32,
  className,
  variant = "beam",
}: Props) {
  // Tier 1 — real photo.
  if (imageUrl) {
    return (
      // eslint-disable-next-line @next/next/no-img-element
      <img
        src={imageUrl}
        alt={`@${username}`}
        width={size}
        height={size}
        className={`proto-avatar ${className ?? ""}`}
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
