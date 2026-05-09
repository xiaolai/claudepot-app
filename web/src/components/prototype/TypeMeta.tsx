import { BookOpen, Star } from "lucide-react";

import type { ToolMeta, PodcastMeta } from "@/lib/prototype-fixtures";
import { formatDuration } from "@/lib/format";
import { markdownToPlaintext } from "@/lib/markdown";

const DISCUSSION_PREVIEW_MAX_CHARS = 220;

export function TutorialMeta({ minutes }: { minutes: number }) {
  return (
    <div className="proto-type-meta">
      <span className="proto-type-meta-pill">
        <BookOpen size={14} aria-hidden /> {minutes} min read
      </span>
    </div>
  );
}

export function PodcastMetaInline({ meta }: { meta: PodcastMeta }) {
  // The previous version rendered a "▶ play" button that didn't
  // actually play anything — visual placeholder, never wired. Inline
  // playback now happens on the post-detail page via UrlAutoEmbed
  // when the submission URL is a Spotify/Apple/YouTube link, so
  // the row meta sticks to text-only metadata.
  return (
    <div className="proto-type-meta">
      <span className="proto-type-meta-pill">{formatDuration(meta.duration_min)}</span>
      <span className="proto-type-meta-host">host: {meta.host}</span>
    </div>
  );
}

export function ToolMetaInline({ meta }: { meta: ToolMeta }) {
  return (
    <div className="proto-type-meta">
      <span className="proto-type-meta-pill">
        <Star size={14} aria-hidden fill="currentColor" /> {meta.stars}
      </span>
      <span className="proto-type-meta-pill">{meta.language}</span>
      <span className="proto-type-meta-host">last commit {meta.last_commit_relative}</span>
    </div>
  );
}

export function DiscussionPreview({ text }: { text: string }) {
  const preview = markdownToPlaintext(text, DISCUSSION_PREVIEW_MAX_CHARS);
  return <p className="proto-discussion-preview">{preview}</p>;
}
