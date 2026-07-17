import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { cache } from "react";
import { CodeCopyEnhancer } from "@/components/prototype/CodeCopyEnhancer";
import { CommentForm } from "@/components/prototype/CommentForm";
import { CommentThread } from "@/components/prototype/CommentThread";
import { MermaidEnhancer } from "@/components/prototype/MermaidEnhancer";
import { SubmissionMeta } from "@/components/prototype/SubmissionMeta";
import { SubmissionStateBanner } from "@/components/prototype/SubmissionStateBanner";
import { UrlAutoEmbed } from "@/components/prototype/UrlAutoEmbed";
import { VoteButtons } from "@/components/prototype/VoteButtons";
import { auth } from "@/lib/auth";
import { totalCommentCount } from "@/lib/format";
import { effectiveDecision, effectiveState } from "@/lib/moderation-fixtures";
import { getCurrentUser, isStaff } from "@/lib/auth-shim";
import {
  getCommentsForSubmission,
  getSubmissionById,
  getViewerVoteForSubmission,
} from "@/db/queries";
import { extractToc, renderMarkdown } from "@/lib/markdown";

const getCachedSubmissionById = cache((id: string) => getSubmissionById(id));

// Show the on-this-page TOC sidebar only when the post has enough
// structure to make navigation worth the column. Below this, the post
// renders in the existing centered narrow column. The threshold is a
// taste call — three is the smallest list where a TOC carries more
// information than the eye can grab from the body itself.
const TOC_MIN_ENTRIES = 3;

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  const { id } = await params;
  const post = await getCachedSubmissionById(id);
  if (!post) return { title: "Not found" };
  // Same visibility gate as the page body below: non-approved
  // submissions expose title/description only to the author and
  // staff. Without this, crawlers and anonymous viewers could read
  // hidden-content metadata that the page itself 404s. (The dev-only
  // `?as=` shim isn't consulted here — metadata for a shimmed viewer
  // just falls back to the generic title.)
  if (effectiveState(post) !== "approved") {
    const session = await auth();
    const viewerUsername = session?.user?.username ?? null;
    const viewerRole = session?.user?.role;
    const canSeeNonApproved =
      viewerUsername === post.user ||
      viewerRole === "staff" ||
      viewerRole === "system";
    if (!canSeeNonApproved) return { title: "Not found" };
  }
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
  const post = await getCachedSubmissionById(id);
  if (!post) notFound();

  const state = effectiveState(post);

  // Real Auth.js session takes precedence; the `?as=` shim is the dev-only
  // fallback. Until phase 3 rips the shim out we merge the two so logged-in
  // users see the comment form even when they aren't using `?as=`.
  const sessionPromise = auth();
  const commentsPromise = getCommentsForSubmission(id);
  const session = await sessionPromise;
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
  const comments = await commentsPromise;
  const score = post.upvotes - post.downvotes;
  const total = totalCommentCount(comments);

  // Look up the viewer's existing vote so VoteButtons renders with
  // the right initialState. Without this the UI thinks every viewer
  // is voting fresh — flipping a real vote produces a server delta
  // of 2 but a UI delta of 1, which surfaces as the score
  // "double-counting" by 1 on the next refresh.
  const viewerVoteValue = session?.user?.id
    ? await getViewerVoteForSubmission(session.user.id, id)
    : null;
  const initialVote: "up" | "down" | null =
    viewerVoteValue === 1 ? "up" : viewerVoteValue === -1 ? "down" : null;
  const bodyHtml = post.text
    ? await renderMarkdown(post.text, { allowMediaEmbeds: true })
    : null;
  const toc = bodyHtml ? extractToc(bodyHtml) : [];
  const showToc = toc.length >= TOC_MIN_ENTRIES;

  // The two layouts share content; only the wrapper class and TOC
  // aside differ. proto-page-aside is the existing pattern from
  // /privacy + /office — sticky sidebar, content column. On mobile
  // the TOC collapses into a tap-to-expand <details> disclosure
  // (proto-toc-details) so a long TOC doesn't push the post body
  // two screens down.
  const articleBody = (
    <>
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
        <VoteButtons
          initialScore={score}
          initialState={initialVote}
          submissionId={post.id}
        />
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
            showBotTail
          />
        </div>
      </div>

      {/* Auto-embed when the URL points at YouTube / Spotify / Apple
          Podcasts. Returns null otherwise; non-platform URLs stay as
          the link in the title row above. */}
      <UrlAutoEmbed url={post.url} />

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
    </>
  );

  if (!showToc) {
    return <div className="proto-page-narrow">{articleBody}</div>;
  }

  return (
    <div className="proto-page-aside">
      <nav className="proto-page-aside-nav" aria-label="On this page">
        <details className="proto-toc-details">
          <summary className="proto-page-aside-nav-title">On this page</summary>
          <ul>
            {toc.map((entry) => (
              <li key={entry.id}>
                <a href={`#${entry.id}`}>{entry.text}</a>
              </li>
            ))}
          </ul>
        </details>
      </nav>
      <div className="proto-page-aside-content">{articleBody}</div>
    </div>
  );
}
