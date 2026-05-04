"use client";

import { useState, useTransition } from "react";
import { Bookmark } from "lucide-react";

import { save as saveAction } from "@/lib/actions/vote";

interface Props {
  initialSaved?: boolean;
  submissionId?: string;
}

export function SaveButton({ initialSaved = false, submissionId }: Props) {
  const [saved, setSaved] = useState(initialSaved);
  const [, startTransition] = useTransition();

  const toggle = () => {
    const next = !saved;
    setSaved(next);
    if (!submissionId) return;
    startTransition(async () => {
      await saveAction({ submissionId, saved: next });
    });
  };

  return (
    <button
      type="button"
      className={`proto-save ${saved ? "active" : ""}`}
      onClick={toggle}
      aria-label={saved ? "Saved (click to remove)" : "Save for later"}
      aria-pressed={saved}
      title={saved ? "Saved — in your inbox" : "Save for later (private)"}
    >
      <Bookmark
        size={16}
        aria-hidden
        fill={saved ? "currentColor" : "none"}
      />
    </button>
  );
}
