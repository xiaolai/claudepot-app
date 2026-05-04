import Link from "next/link";
import { ExternalLink } from "lucide-react";
import { VoteButtons } from "./VoteButtons";
import { SaveButton } from "./SaveButton";
import { SubmissionMeta } from "./SubmissionMeta";
import { SubmissionStateBanner } from "./SubmissionStateBanner";
import { TutorialMeta, PodcastMetaInline, ToolMetaInline, DiscussionPreview } from "./TypeMeta";
import type { Submission } from "@/lib/prototype-fixtures";
import { effectiveState, effectiveDecision } from "@/lib/moderation";

interface Props {
  rank?: number;
  submission: Submission;
  initialVote?: "up" | "down" | null;
  initialSaved?: boolean;
  compact?: boolean;
  /** When true, render pending/rejected rows with their state banner. Default true. */
  showState?: boolean;
}

export function SubmissionRow({
  rank,
  submission: s,
  initialVote = null,
  initialSaved = false,
  compact = false,
  showState = true,
}: Props) {
  const score = s.upvotes - s.downvotes;
  const state = effectiveState(s);
  const decision = state !== "approved" ? effectiveDecision(s) : null;
  const tags = state === "pending" && decision ? decision.tags_assigned : s.tags;

  return (
    <li
      className={`proto-row ${compact ? "proto-row-compact" : ""} proto-row-state-${state}`}
    >
      {rank !== undefined && <span className="proto-rank">{rank}.</span>}
      <VoteButtons
        initialScore={score}
        initialState={initialVote}
        submissionId={s.id}
      />
      <div className="proto-row-content">
        {showState && decision && (
          <SubmissionStateBanner
            state={state}
            decision={decision}
            variant="row"
            submissionId={s.id}
          />
        )}

        <h3 className="proto-row-title">
          <Link href={`/post/${s.id}`}>{s.title}</Link>
          {s.url && (
            <a
              href={s.url}
              target="_blank"
              rel="noopener noreferrer"
              className="proto-row-source"
              aria-label={`Open ${s.domain} in a new tab`}
            >
              {s.domain}
              <ExternalLink size={12} aria-hidden />
            </a>
          )}
        </h3>

        {!compact && s.type === "tool" && s.tool_meta && (
          <ToolMetaInline meta={s.tool_meta} />
        )}
        {!compact && s.type === "podcast" && s.podcast_meta && (
          <PodcastMetaInline meta={s.podcast_meta} />
        )}
        {!compact && s.type === "tutorial" && s.reading_time_min && (
          <TutorialMeta minutes={s.reading_time_min} />
        )}
        {!compact && s.type === "discussion" && s.text && (
          <DiscussionPreview text={s.text} />
        )}

        <SubmissionMeta
          submission={s}
          showCommentCount={state === "approved"}
          tags={tags}
        />
      </div>
      <SaveButton initialSaved={initialSaved} submissionId={s.id} />
    </li>
  );
}
