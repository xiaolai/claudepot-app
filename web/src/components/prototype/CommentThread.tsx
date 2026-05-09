import Link from "next/link";
import { CircleDashed, X } from "lucide-react";
import type { CommentNode } from "@/lib/prototype-fixtures";
import { commentEffectiveState } from "@/lib/moderation-fixtures";
import { relativeTime } from "@/lib/format";
import { renderMarkdown } from "@/lib/markdown";
import { UserAvatar } from "./Avatar";

async function CommentItem({ node }: { node: CommentNode }) {
  const state = commentEffectiveState(node);

  if (state === "rejected") {
    return (
      <article
        id={`comment-${node.id}`}
        className="proto-comment proto-comment-rejected"
      >
        <header className="proto-comment-meta">
          <span className="proto-state-pill proto-state-pill-rejected">
            <X size={12} aria-hidden /> Removed
          </span>
          <span className="proto-state-note">
            {node.ai_decision?.reason ?? "Removed by AI moderation."}
          </span>
        </header>
      </article>
    );
  }

  const bodyHtml = node.tombstoned ? null : await renderMarkdown(node.body);

  return (
    <article
      id={`comment-${node.id}`}
      className={`proto-comment ${state === "pending" ? "proto-comment-pending" : ""}`}
    >
      {state === "pending" && (
        <header className="proto-comment-meta">
          <span className="proto-state-pill proto-state-pill-pending">
            <CircleDashed size={12} aria-hidden /> Under AI review
          </span>
          {node.ai_decision && (
            <span className="proto-state-note">
              Confidence {Math.round(node.ai_decision.confidence * 100)}%
            </span>
          )}
        </header>
      )}
      <header className="proto-comment-meta proto-comment-meta-with-avatar">
        <UserAvatar
          username={node.user}
          imageUrl={node.user_image_url}
          size={20}
        />
        <Link href={`/u/${node.user}`}>{node.user}</Link>
        {node.user_is_agent && (
          <span
            className="proto-ai-chip"
            title="Authored by an AI agent"
            aria-label="AI agent"
          >
            AI
          </span>
        )}
        {" · "}
        <span>{relativeTime(node.submitted_at)}</span>
        {node.updated_at && (
          <>
            {" · "}
            <span
              title={`Edited ${new Date(node.updated_at).toLocaleString()}`}
            >
              edited {relativeTime(node.updated_at)}
            </span>
          </>
        )}
        {/* render-if-nonzero per design.md — `0 pts` is noise. */}
        {node.upvotes - node.downvotes !== 0 && (
          <>
            {" · "}
            <span>{node.upvotes - node.downvotes} pts</span>
          </>
        )}
      </header>
      {bodyHtml === null ? (
        <div className="proto-comment-body proto-comment-body-tombstone">
          [deleted]
        </div>
      ) : (
        <div
          className="proto-comment-body"
          dangerouslySetInnerHTML={{ __html: bodyHtml }}
        />
      )}
      {node.children.length > 0 && (
        <div className="proto-comment-children">
          {node.children.map((c) => (
            <CommentItem key={c.id} node={c} />
          ))}
        </div>
      )}
    </article>
  );
}

export function CommentThread({ nodes }: { nodes: CommentNode[] }) {
  if (nodes.length === 0) {
    return <p className="proto-empty">No comments yet.</p>;
  }
  return (
    <div>
      {nodes.map((c) => (
        <CommentItem key={c.id} node={c} />
      ))}
    </div>
  );
}
