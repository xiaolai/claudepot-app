"use client";

import { useState, useTransition } from "react";

import { followTag } from "@/lib/actions/tag";

/**
 * Follow / unfollow toggle for a tag. Optimistic on success: flips
 * the local state immediately; the server action also runs
 * revalidatePath on the tag page so a hard refresh re-syncs.
 *
 * Anonymous viewers see a sign-in prompt; signed-in viewers see the
 * Follow / Following toggle. The page passes `signedIn` from the
 * server-side auth() call so we don't need a client-side session
 * fetch.
 */
export function FollowTagButton({
  tagSlug,
  initialFollowed,
  signedIn,
}: {
  tagSlug: string;
  initialFollowed: boolean;
  signedIn: boolean;
}) {
  const [followed, setFollowed] = useState(initialFollowed);
  const [pending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);

  if (!signedIn) {
    return (
      <a href={`/login?callbackUrl=${encodeURIComponent(`/c/${tagSlug}`)}`}
         className="proto-btn-secondary">
        Sign in to follow
      </a>
    );
  }

  function toggle() {
    setError(null);
    const next = !followed;
    setFollowed(next);
    startTransition(async () => {
      try {
        const result = await followTag({ tagSlug, followed: next });
        if (!result.ok) {
          setFollowed(!next);
          setError(
            result.reason === "unauth"
              ? "Sign in first."
              : result.reason === "unavailable"
                ? "Follow isn't available yet on this deployment."
                : "Couldn't update. Try again.",
          );
        }
      } catch {
        // Server action threw (e.g. DB exception, network drop). Roll
        // the optimistic flip back so the UI matches reality.
        setFollowed(!next);
        setError("Couldn't update. Try again.");
      }
    });
  }

  return (
    <span className="proto-tag-follow">
      <button
        type="button"
        className={followed ? "proto-btn-secondary" : "proto-btn-primary"}
        onClick={toggle}
        disabled={pending}
        aria-pressed={followed}
      >
        {pending
          ? followed
            ? "Following…"
            : "Unfollowing…"
          : followed
            ? "Following"
            : "Follow"}
      </button>
      {error ? (
        <span className="proto-form-flash proto-form-flash-err">{error}</span>
      ) : null}
    </span>
  );
}
