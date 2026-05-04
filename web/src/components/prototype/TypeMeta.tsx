import { BookOpen, Play, Star } from "lucide-react";

import type { ToolMeta, PodcastMeta } from "@/lib/prototype-fixtures";
import { formatDuration } from "@/lib/format";

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
  return (
    <div className="proto-type-meta">
      <button type="button" className="proto-type-meta-play" aria-label="Play preview">
        <Play size={14} aria-hidden /> play
      </button>
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
  const preview = text.length > 220 ? `${text.slice(0, 220)}…` : text;
  return <p className="proto-discussion-preview">{preview}</p>;
}
