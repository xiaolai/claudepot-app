/**
 * YouTube embed pre-pass for the markdown renderer.
 *
 * Two trigger shapes, both run BEFORE marked.parse so the rewritten
 * iframe HTML lands in the marked input as a raw HTML block:
 *
 *   1. Bare URL paragraph — a paragraph that consists of nothing but
 *      a YouTube URL (any of the five canonical shapes: watch,
 *      youtu.be, shorts, live, embed). Reddit/Discourse Onebox
 *      precedent — what users naturally do.
 *
 *   2. Directive shortcode — `:youtube[ID]` on its own line. Borrowed
 *      from the CommonMark generic-directive proposal so it reads
 *      natively to anyone who's seen :cite[…] / :note[…] etc.
 *      Non-paragraph contexts (e.g. between two prose paragraphs in
 *      one block of text) get this; pure URL paste gets shape 1.
 *
 * Defense in depth: every emitted iframe has src locked to
 * https://www.youtube-nocookie.com/embed/<11-char-id>. The
 * sanitize-html step in markdown.ts re-validates this src via
 * transformTags.iframe — if the regex below ever drifts the
 * sanitizer drops the embed entirely rather than emit a bad iframe.
 */

const VIDEO_ID = /^[a-zA-Z0-9_-]{11}$/;

/**
 * Extract a YouTube video ID from a URL string. Returns null if the
 * URL doesn't match a recognised YouTube shape or the ID isn't the
 * canonical 11-character form.
 *
 * Recognised shapes:
 *   - youtube.com/watch?v=ID                 (and m.youtube.com)
 *   - youtu.be/ID
 *   - youtube.com/shorts/ID
 *   - youtube.com/live/ID
 *   - youtube.com/embed/ID
 *   - youtube-nocookie.com/embed/ID
 *
 * Trailing query strings on path-segment shapes are tolerated and
 * dropped; the `v` param on watch URLs is honoured. Other params
 * (t=, list=, …) are dropped — we don't carry them through the
 * embed because most are user-tracking surface.
 */
export function extractYoutubeId(url: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const host = parsed.hostname.replace(/^www\./, "").replace(/^m\./, "");

  if (host === "youtu.be") {
    const id = parsed.pathname.slice(1).split("/")[0];
    return VIDEO_ID.test(id) ? id : null;
  }

  if (host === "youtube.com" || host === "youtube-nocookie.com") {
    if (parsed.pathname === "/watch") {
      const id = parsed.searchParams.get("v") ?? "";
      return VIDEO_ID.test(id) ? id : null;
    }
    const m = parsed.pathname.match(
      /^\/(?:shorts|live|embed)\/([a-zA-Z0-9_-]+)/,
    );
    if (m && VIDEO_ID.test(m[1])) return m[1];
  }

  return null;
}

/** Build the iframe block for a given video id. */
function buildEmbed(id: string): string {
  const safeId = id; // already validated by VIDEO_ID
  const src = `https://www.youtube-nocookie.com/embed/${safeId}`;
  // The wrapper carries the responsive aspect-ratio in CSS
  // (.proto-yt-embed) so the iframe itself can be width/height 100%
  // and adapt to the column. allowfullscreen is a boolean attribute
  // — sanitize-html keeps it as long as the attribute name is
  // allowed, regardless of value.
  return (
    `<div class="proto-yt-embed">` +
    `<iframe src="${src}"` +
    ` title="YouTube video"` +
    ` loading="lazy"` +
    ` referrerpolicy="strict-origin-when-cross-origin"` +
    ` sandbox="allow-scripts allow-same-origin allow-presentation"` +
    ` allow="accelerometer; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"` +
    ` allowfullscreen` +
    `></iframe>` +
    `</div>`
  );
}

/**
 * Pre-process a markdown source string, rewriting recognised
 * YouTube triggers into raw-HTML iframe blocks. Idempotent: the
 * emitted blocks don't re-match either trigger.
 *
 * Apply this BEFORE marked.parse so the iframe lands in the marked
 * input as a raw HTML block (the only safe insertion point that
 * doesn't get treated as inline text).
 */
export function rewriteYoutubeEmbeds(source: string): string {
  let out = source;

  // Shortcode: `:youtube[ID]` on its own line. Allow optional
  // surrounding whitespace; require flanking newlines (or BOF/EOF)
  // so the directive is paragraph-alone.
  out = out.replace(
    /(^|\n)[ \t]*:youtube\[([a-zA-Z0-9_-]{11})\][ \t]*(?=\n|$)/g,
    (_match, lead: string, id: string) => `${lead}\n${buildEmbed(id)}\n`,
  );

  // Bare URL paragraph: a line that contains exactly one URL whose
  // host matches our YT family. Strict — any prose on the same line
  // disqualifies, matching the Reddit/Discourse rule.
  out = out.replace(
    /(^|\n)[ \t]*(https?:\/\/\S+)[ \t]*(?=\n|$)/g,
    (match, lead: string, url: string) => {
      const id = extractYoutubeId(url);
      if (!id) return match;
      return `${lead}\n${buildEmbed(id)}\n`;
    },
  );

  return out;
}
