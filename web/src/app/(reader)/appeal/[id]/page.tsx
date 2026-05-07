import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { policyDecisions, submissions } from "@/db/schema";
import { auth } from "@/lib/auth";
import { relativeTime } from "@/lib/format";
import { AppealForm } from "./AppealForm";

/**
 * Author-facing appeal page for a moderator reject.
 *
 * Renders the verdict (category, one-line-why, when), the original
 * content (read-only), and the AppealForm (which posts to the
 * submitAppeal server action).
 *
 * 404s for any user who is not the original author. Redirects to
 * /login if unauthenticated.
 */

const CATEGORY_LABELS: Record<string, string> = {
  spam: "Spam",
  abuse: "Abuse",
  illegal: "Illegal content",
  doxxing: "Doxxing",
  off_topic: "Off-topic",
};

export default async function AppealPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;

  const session = await auth();
  if (!session?.user?.id) {
    redirect(`/login?callbackUrl=${encodeURIComponent(`/appeal/${id}`)}`);
  }

  const [decision] = await db
    .select({
      id: policyDecisions.id,
      authorId: policyDecisions.authorId,
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      verdict: policyDecisions.verdict,
      category: policyDecisions.category,
      oneLineWhy: policyDecisions.oneLineWhy,
      decidedAt: policyDecisions.decidedAt,
    })
    .from(policyDecisions)
    .where(eq(policyDecisions.id, id))
    .limit(1);

  if (!decision) notFound();
  if (decision.authorId !== session.user.id) notFound();
  if (decision.verdict !== "reject") notFound();

  let originalTitle: string | null = null;
  let originalBody: string | null = null;
  let submissionState: string | null = null;
  if (decision.targetType === "submission" && decision.targetId) {
    const [sub] = await db
      .select({
        title: submissions.title,
        text: submissions.text,
        url: submissions.url,
        state: submissions.state,
      })
      .from(submissions)
      .where(eq(submissions.id, decision.targetId))
      .limit(1);
    if (sub) {
      originalTitle = sub.title;
      originalBody = sub.text ?? sub.url ?? null;
      submissionState = sub.state;
    }
  }

  const stillAppealable =
    decision.targetType === "comment"
      ? decision.targetId !== null
      : submissionState === "rejected";

  return (
    <div className="proto-page-narrow">
      <h1>Appeal a moderation decision</h1>
      <p className="proto-dek">
        The AI policy moderator rejected your{" "}
        {decision.targetType === "comment" ? "comment" : "submission"}. If you
        think it got it wrong, send an appeal — staff reviews these directly.
      </p>

      <section className="proto-appeal-verdict">
        <h2>The decision</h2>
        <dl>
          <dt>Category</dt>
          <dd>
            {decision.category
              ? (CATEGORY_LABELS[decision.category] ?? decision.category)
              : "—"}
          </dd>
          <dt>Reason</dt>
          <dd>{decision.oneLineWhy}</dd>
          <dt>When</dt>
          <dd>{relativeTime(decision.decidedAt.toISOString())}</dd>
        </dl>
      </section>

      {originalTitle ? (
        <section className="proto-appeal-content">
          <h2>Your submission</h2>
          <h3>{originalTitle}</h3>
          {originalBody ? (
            <pre className="proto-appeal-content-body">{originalBody}</pre>
          ) : null}
        </section>
      ) : null}

      {stillAppealable ? (
        <section className="proto-appeal-action">
          <h2>Submit an appeal</h2>
          <AppealForm decisionId={decision.id} />
        </section>
      ) : (
        <section className="proto-appeal-action">
          <p className="proto-empty">
            This decision is no longer appealable — the content has already
            been approved or deleted.{" "}
            <Link href="/notifications">Back to notifications</Link>.
          </p>
        </section>
      )}
    </div>
  );
}
