"use client";

import { useState, useTransition } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";

import { vote as voteAction } from "@/lib/actions/vote";

interface Props {
  initialScore: number;
  initialState?: "up" | "down" | null;
  submissionId?: string;
}

export function VoteButtons({
  initialScore,
  initialState = null,
  submissionId,
}: Props) {
  const [vote, setVote] = useState<"up" | "down" | null>(initialState);
  const [, startTransition] = useTransition();
  const [karmaGate, setKarmaGate] = useState(false);

  const baseAdjustment =
    initialState === "up" ? -1 : initialState === "down" ? 1 : 0;
  const currentAdjustment = vote === "up" ? 1 : vote === "down" ? -1 : 0;
  const displayScore = initialScore + baseAdjustment + currentAdjustment;

  const flip = (next: "up" | "down") => {
    const newState = vote === next ? null : next;
    setVote(newState);
    setKarmaGate(false);

    if (!submissionId) return; // standalone preview, no DB write
    const value = newState === "up" ? 1 : newState === "down" ? -1 : 0;
    startTransition(async () => {
      const result = await voteAction({ submissionId, value });
      if (!result.ok && result.reason === "karma_gate") {
        setVote(null);
        setKarmaGate(true);
      }
    });
  };

  return (
    <span className="proto-vote">
      <button
        type="button"
        className={`proto-vote-arrow proto-vote-up ${vote === "up" ? "active" : ""}`}
        onClick={() => flip("up")}
        aria-label="Upvote (public signal)"
        aria-pressed={vote === "up"}
      >
        <ChevronUp size={16} aria-hidden />
      </button>
      <span className="proto-vote-score">{displayScore}</span>
      <button
        type="button"
        className={`proto-vote-arrow proto-vote-down ${vote === "down" ? "active" : ""}`}
        onClick={() => flip("down")}
        aria-label={
          karmaGate
            ? "Earn 100 karma to downvote"
            : "Downvote"
        }
        aria-pressed={vote === "down"}
        title={karmaGate ? "Earn 100 karma to downvote" : undefined}
      >
        <ChevronDown size={16} aria-hidden />
      </button>
    </span>
  );
}
