import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { CodeCopyEnhancer } from "@/components/prototype/CodeCopyEnhancer";
import { CommentForm } from "@/components/prototype/CommentForm";
import { CommentThread } from "@/components/prototype/CommentThread";
import { MermaidEnhancer } from "@/components/prototype/MermaidEnhancer";
import { SubmissionMeta } from "@/components/prototype/SubmissionMeta";
import { SubmissionStateBanner } from "@/components/prototype/SubmissionStateBanner";
import { VoteButtons } from "@/components/prototype/VoteButtons";
import { auth } from "@/lib/auth";
import { totalCommentCount } from "@/lib/format";
import { effectiveDecision, effectiveState } from "@/lib/moderation";
import { getCurrentUser, isStaff } from "@/lib/auth-shim";
import { getCommentsForSubmission, getSubmissionById } from "@/db/queries";
import { renderMarkdown } from "@/lib/markdown";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  const { id } = await params;
  const post = await getSubmissionById(id);
  if (!post) return { title: "Not found" };
  return {
    title: post.title,
    openGraph: {
      title: post.title,
      description: post.text?.slice(0, 200) ?? `by @${post.user}`,
      images: [`/api/og/submission/${id}`],
    },
    twitter: {
      card: "summary_large_image",
      title: post.title,
      images: [`/api/og/submission/${id}`],
    },
  };
}

export default async function PostDetail({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ as?: string }>;
}) {
  const { id } = await params;
  const sp = await searchParams;
  const post = await getSubmissionById(id);
  if (!post) notFound();

  const state = effectiveState(post);

  // Real Auth.js session takes precedence; the `?as=` shim is the dev-only
  // fallback. Until phase 3 rips the shim out we merge the two so logged-in
  // users see the comment form even when they aren't using `?as=`.
  const session = await auth();
  const devUser = getCurrentUser(sp);
  const viewerUsername = session?.user?.username ?? devUser?.username ?? null;
  const viewerRole = session?.user?.role;
  const isAuthor = viewerUsername === post.user;
  const isStaffViewer =
    viewerRole === "staff" || viewerRole === "system" || isStaff(devUser);
  const isSignedIn = Boolean(session?.user || devUser);
  const canSeeNonApproved = isAuthor || isStaffViewer;

  if (state !== "approved" && !canSeeNonApproved) notFound();

  const decision = state !== "approved" ? effectiveDecision(post) : null;
  const comments = await getCommentsForSubmission(id);
  const score = post.upvotes - post.downvotes;
  const total = totalCommentCount(comments);
  const bodyHtml = post.text ? await renderMarkdown(post.text) : null;

  return (
    <div className="proto-page-narrow">
      {decision && (
        <SubmissionStateBanner
          state={state}
          decision={decision}
          variant="detail"
          submissionId={post.id}
          submittedAt={post.submitted_at}
        />
      )}

      <div className="proto-row proto-row-bare">
        <VoteButtons initialScore={score} submissionId={post.id} />
        <div className="proto-row-content">
          <h1 className="proto-row-title">
            {post.url ? (
              <a href={post.url} target="_blank" rel="noopener noreferrer">
                {post.title}
              </a>
            ) : (
              post.title
            )}
            {post.url && <span className="proto-row-domain">({post.domain})</span>}
          </h1>
          <SubmissionMeta
            submission={post}
            showCommentCount={state === "approved"}
            commentCount={total}
            linkComments={false}
          />
        </div>
      </div>

      {bodyHtml && (
        <div
          className="proto-text"
          dangerouslySetInnerHTML={{ __html: bodyHtml }}
        />
      )}

      {state === "approved" && (
        <section className="proto-comments" aria-label="Comments">
          {isSignedIn ? (
            <CommentForm submissionId={post.id} />
          ) : (
            <div className="proto-comment-box">
              <Link href="/login">Sign in</Link> to comment.
            </div>
          )}
          <CommentThread nodes={comments} />
        </section>
      )}

      {/* Progressive enhancement for ```mermaid``` blocks in the body
       * and comments. Mounts once; finds and renders all diagrams in
       * one pass. The mermaid bundle (~250 KB) is dynamically imported
       * only when at least one diagram is present on the page. */}
      <MermaidEnhancer />
      {/* Wires the SSR'd copy buttons in code blocks to the clipboard.
       * Pure click handler — the button shell, lucide SVG, and line
       * gutter are all baked into the HTML by decorateCodeBlocks. */}
      <CodeCopyEnhancer />
    </div>
  );
}
