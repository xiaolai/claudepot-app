"use client";

import { useState, useTransition } from "react";

import { submitAppeal, type AppealResult } from "@/lib/actions/appeals";

type Props = {
  decisionId: string;
};

/**
 * Author-facing appeal form. The full policy decision is rendered
 * by the parent server component; this client component owns only
 * the textarea + submit + status feedback.
 */
export function AppealForm({ decisionId }: Props) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState(false);
  const [isPending, startTransition] = useTransition();

  function handleSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    startTransition(async () => {
      const res: AppealResult = await submitAppeal({ decisionId, text });
      if (!res.ok) {
        setError(formatReason(res.reason));
        return;
      }
      setSubmitted(true);
      setText("");
    });
  }

  if (submitted) {
    return (
      <div className="proto-appeal-success" role="status">
        Your appeal has been submitted. Staff review the queue daily; you&rsquo;ll
        get a notification when it&rsquo;s decided.
      </div>
    );
  }

  return (
    <form onSubmit={handleSubmit} className="proto-appeal-form">
      <label htmlFor="appeal-text">Your appeal</label>
      <textarea
        id="appeal-text"
        name="text"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={6}
        minLength={10}
        maxLength={480}
        required
        disabled={isPending}
        placeholder="Tell us why you think the decision was wrong. 10–480 characters."
      />
      <div className="proto-appeal-form-actions">
        <button
          type="submit"
          className="proto-btn-primary"
          disabled={isPending || text.trim().length < 10}
        >
          {isPending ? "Submitting…" : "Submit appeal"}
        </button>
        {error ? (
          <span className="proto-appeal-error" role="alert">
            {error}
          </span>
        ) : null}
      </div>
    </form>
  );
}

function formatReason(
  reason:
    | "unauth"
    | "not_found"
    | "forbidden"
    | "validation"
    | "duplicate"
    | "stale",
): string {
  switch (reason) {
    case "unauth":
      return "Sign in to appeal.";
    case "not_found":
      return "This decision no longer exists.";
    case "forbidden":
      return "You can only appeal your own content.";
    case "validation":
      return "Please write 10–480 characters explaining the appeal.";
    case "duplicate":
      return "An appeal for this content is already in the queue.";
    case "stale":
      return "This decision is no longer appealable (already approved or deleted).";
  }
}
