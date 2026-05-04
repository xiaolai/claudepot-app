import Link from "next/link";
import { CircleDashed, X } from "lucide-react";
import type { AIDecision, ModerationState } from "@/lib/moderation";
import { relativeTime } from "@/lib/format";

interface Props {
  state: ModerationState;
  decision: AIDecision;
  variant: "row" | "detail";
  submissionId: string;
  /** Detail variant only — the submitted-at timestamp shown in the body copy. */
  submittedAt?: string;
}

/**
 * Renders the AI-moderation banner for a submission. Two variants share
 * the same logic (state → message → confidence) but render different
 * shells: a small inline pill row for feed-style lists, and a full
 * banner for the post-detail page. Returns null for `approved` so
 * callers can drop it in unconditionally.
 */
export function SubmissionStateBanner({
  state,
  decision,
  variant,
  submissionId,
  submittedAt,
}: Props) {
  if (state === "approved") return null;

  if (variant === "row") {
    if (state === "pending") {
      return (
        <div className="proto-row-pending" role="status">
          <span className="proto-state-pill proto-state-pill-pending">
            <CircleDashed size={12} aria-hidden /> Under AI review
          </span>
          <span className="proto-state-note">
            {decision.confidence < 0.85
              ? "Confidence below threshold — routed to human queue."
              : "Awaiting AI decision."}
          </span>
        </div>
      );
    }
    return (
      <div className="proto-row-rejected" role="status">
        <span className="proto-state-pill proto-state-pill-rejected">
          <X size={12} aria-hidden /> Removed by AI moderation
        </span>
        <span className="proto-state-note">{decision.reason}</span>
        <Link href={`/post/${submissionId}`} className="proto-state-appeal">
          Appeal →
        </Link>
      </div>
    );
  }

  if (state === "pending") {
    return (
      <div
        className="proto-post-state-banner proto-post-state-banner-pending"
        role="status"
      >
        <h2 className="proto-post-state-title">
          <CircleDashed size={16} aria-hidden /> AI is reviewing this post
        </h2>
        <p className="proto-post-state-body">
          Your post entered the moderation queue{" "}
          {submittedAt ? relativeTime(submittedAt) : "moments"} ago. Current
          AI confidence:{" "}
          <strong>{Math.round(decision.confidence * 100)}%</strong> —{" "}
          {decision.confidence < 0.85
            ? "below the auto-publish threshold, routed to staff for review."
            : "awaiting final classification."}
        </p>
        <p className="proto-post-state-body">
          <strong>AI note:</strong> {decision.reason}
        </p>
      </div>
    );
  }

  return (
    <div
      className="proto-post-state-banner proto-post-state-banner-rejected"
      role="status"
    >
      <h2 className="proto-post-state-title">
        <X size={16} aria-hidden /> Removed by AI moderation
      </h2>
      <p className="proto-post-state-body">{decision.reason}</p>
      <p className="proto-post-state-body">
        Confidence{" "}
        <strong>{Math.round(decision.confidence * 100)}%</strong> ·{" "}
        <Link href={`/post/${submissionId}#appeal`}>Appeal this decision</Link>
      </p>
    </div>
  );
}
