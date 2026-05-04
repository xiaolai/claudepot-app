"use client";

import { useState, useTransition } from "react";

import { flag as flagAction } from "@/lib/actions/moderation";

interface Props {
  targetType: "submission" | "comment";
  targetId: string;
}

export function FlagButton({ targetType, targetId }: Props) {
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [pending, startTransition] = useTransition();
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (reason.trim().length < 3) {
      setError("Reason needs at least 3 characters.");
      return;
    }
    setError(null);
    startTransition(async () => {
      const result = await flagAction({ targetType, targetId, reason: reason.trim() });
      if (result.ok) {
        setDone(true);
        setReason("");
        setTimeout(() => {
          setOpen(false);
          setDone(false);
        }, 1500);
      } else {
        setError(
          result.reason === "unauth"
            ? "Sign in to flag."
            : result.reason === "unverified"
              ? "Verify your email first."
              : "Couldn't submit flag.",
        );
      }
    });
  };

  if (!open) {
    return (
      <button
        type="button"
        className="proto-mod-btn"
        onClick={() => setOpen(true)}
        aria-label="Flag this content"
      >
        ⚑ flag
      </button>
    );
  }

  return (
    <form className="proto-form proto-form-inline" onSubmit={submit}>
      <input
        type="text"
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        placeholder="Why? (spam, off-topic, etc.)"
        maxLength={500}
        autoFocus
        className="proto-input proto-input-wide"
      />
      <button type="submit" disabled={pending} className="proto-mod-btn proto-mod-btn-warn">
        {pending ? "Sending…" : "Submit"}
      </button>
      <button
        type="button"
        className="proto-mod-btn"
        onClick={() => {
          setOpen(false);
          setError(null);
        }}
      >
        Cancel
      </button>
      {error && <span className="proto-state-pill proto-state-pill-rejected">{error}</span>}
      {done && <span className="proto-state-pill proto-state-pill-pending">Flag submitted.</span>}
    </form>
  );
}
