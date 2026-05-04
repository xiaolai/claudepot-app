"use client";

import { useState, useTransition } from "react";

import { submitComment } from "@/lib/actions/comment";
import { MarkdownEditor } from "./MarkdownEditor";

interface Props {
  submissionId: string;
  parentId?: string | null;
  onSubmitted?: () => void;
  placeholder?: string;
  compact?: boolean;
}

export function CommentForm({
  submissionId,
  parentId = null,
  onSubmitted,
  placeholder = "Write a comment…",
  compact = false,
}: Props) {
  const [body, setBody] = useState("");
  const [pending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setInfo(null);
    const text = body.trim();
    if (text.length < 2) {
      setError("Comments need at least 2 characters.");
      return;
    }
    startTransition(async () => {
      const result = await submitComment({
        submissionId,
        parentId,
        body: text,
      });
      if (!result.ok) {
        setError(
          result.reason === "unauth"
            ? "Sign in to comment."
            : result.reason === "locked"
              ? "Your account is locked."
              : "Couldn't post — try again.",
        );
        return;
      }
      setBody("");
      setInfo(
        result.pending
          ? "Submitted — your comment is being reviewed."
          : null,
      );
      onSubmitted?.();
    });
  };

  return (
    <form
      className={`proto-form ${compact ? "proto-form-inline" : ""}`}
      onSubmit={handleSubmit}
    >
      <MarkdownEditor
        rows={compact ? 3 : 4}
        value={body}
        onChange={setBody}
        placeholder={placeholder}
        disabled={pending}
        maxLength={40000}
      />
      <div className="proto-row-meta">
        <button
          type="submit"
          className="proto-btn-primary"
          disabled={pending || body.trim().length < 2}
        >
          {pending ? "Posting…" : "Post"}
        </button>
        {error && <span className="proto-state-pill proto-state-pill-rejected">{error}</span>}
        {info && <span className="proto-state-pill proto-state-pill-pending">{info}</span>}
      </div>
    </form>
  );
}
