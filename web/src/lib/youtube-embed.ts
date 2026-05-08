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
 *
 * Context-aware: walks the source line by line and skips lines
 * inside fenced code blocks (``` or ~~~) so a YouTube URL inside a
 * code sample is preserved verbatim. Indented code blocks (4+ space
 * indent) are skipped via the 0-3-space leading-indent gate on the
 * trigger regexes — same threshold CommonMark uses.
 */
export function rewriteYoutubeEmbeds(source: string): string {
  const lines = source.split("\n");
  const out: string[] = [];
  let inFence = false;
  let fenceChar: "`" | "~" | null = null;
  let fenceLen = 0;

  // Trigger regexes — both require 0-3 leading spaces (any more
  // would be an indented code block in CommonMark).
  const URL_LINE = /^ {0,3}(https?:\/\/\S+)[ \t]*$/;
  const DIRECTIVE_LINE = /^ {0,3}:youtube\[([a-zA-Z0-9_-]{11})\][ \t]*$/;

  for (const line of lines) {
    // Fence open/close detection. Per CommonMark, a fence opener is
    // a line starting with 0-3 spaces followed by 3+ backticks (or
    // tildes) and an optional info string; the matching closer must
    // use the same character and at least the same length.
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

    // Directive shortcode first — more specific match wins.
    const directiveMatch = line.match(DIRECTIVE_LINE);
    if (directiveMatch) {
      out.push(buildEmbed(directiveMatch[1]));
      continue;
    }

    // Bare URL paragraph.
    const urlMatch = line.match(URL_LINE);
    if (urlMatch) {
      const id = extractYoutubeId(urlMatch[1]);
      if (id) {
        out.push(buildEmbed(id));
        continue;
      }
    }

    out.push(line);
  }

  return out.join("\n");
}
