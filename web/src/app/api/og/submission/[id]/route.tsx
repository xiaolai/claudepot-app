import { ImageResponse } from "next/og";

import { getSubmissionById } from "@/db/queries";

export const runtime = "nodejs";

// Raster image output. The design-token system applies to the web UI,
// not to PNG generation by next/og — these px values are layout
// constants for the 1200×630 OpenGraph card, not interactive UI.

const OG_WIDTH = 1200;
const OG_HEIGHT = 630;
const PADDING = 80;
const TITLE_SIZE = 60;
const META_SIZE = 24;
const EYEBROW_SIZE = 20;
const TITLE_MAX_HEIGHT = 350;
const BG_GRADIENT = "linear-gradient(135deg, #fff 0%, #f5e6d8 100%)";
const ACCENT_INK = "#a35a2a";
const TEXT_DARK = "#1a1a2e";
const META_MUTED = "#6b7280";

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ id: string }> },
) {
  const { id } = await params;
  const post = await getSubmissionById(id);

  const title = post?.title ?? "ClauDepot";
  const author = post ? `@${post.user}` : "ClauDepot";
  const score = post?.upvotes ? post.upvotes - post.downvotes : 0;

  return new ImageResponse(
    (
      <div
        style={{
          width: `${OG_WIDTH}px`,
          height: `${OG_HEIGHT}px`,
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          padding: `${PADDING}px`,
          background: BG_GRADIENT,
          fontFamily: "serif",
        }}
      >
        <div
          style={{
            fontSize: `${EYEBROW_SIZE}px`,
            color: ACCENT_INK,
            letterSpacing: "0.1em",
            textTransform: "uppercase",
          }}
        >
          ClauDepot
        </div>
        <div
          style={{
            fontSize: `${TITLE_SIZE}px`,
            color: TEXT_DARK,
            lineHeight: 1.15,
            display: "block",
            maxHeight: `${TITLE_MAX_HEIGHT}px`,
            overflow: "hidden",
          }}
        >
          {title}
        </div>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            fontSize: `${META_SIZE}px`,
            color: META_MUTED,
          }}
        >
          <span>{author}</span>
          <span>{score} points</span>
        </div>
      </div>
    ),
    { width: OG_WIDTH, height: OG_HEIGHT },
  );
}
