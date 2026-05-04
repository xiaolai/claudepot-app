"use client";

import { useState, useTransition } from "react";

import { renameUsername } from "@/lib/actions/username";
import {
  MAX_SELF_RENAMES,
  SELF_RENAME_COOLDOWN_MINUTES,
  SELF_RENAME_GRACE_DAYS,
  type SelfRenameDecision,
} from "@/lib/username";

type Props = {
  currentUsername: string;
  decision: SelfRenameDecision;
  renamesUsed: number;
};

/**
 * Username display + in-grace rename form.
 *
 * The grace decision is computed server-side and passed down so the
 * UI can either expose a rename form or explain why it can't. The
 * form's optimistic disabled-state is enough on the client; the
 * server action re-evaluates eligibility under the DB clock so a
 * stale tab can't bypass the cooldown.
 */
export function UsernamePanel({
  currentUsername,
  decision,
  renamesUsed,
}: Props) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(currentUsername);
  const [error, setError] = useState<string | null>(null);
  const [isPending, startTransition] = useTransition();

  function reasonLine(d: SelfRenameDecision): string | null {
    if (d.ok) return null;
    switch (d.reason) {
      case "grace_expired":
        return `Grace window ended after ${SELF_RENAME_GRACE_DAYS} days. Contact staff to change.`;
      case "count_exceeded":
        return `All ${MAX_SELF_RENAMES} changes used.`;
      case "cooldown":
        return `Wait ${SELF_RENAME_COOLDOWN_MINUTES} minutes between changes.`;
    }
  }

  function onSubmit(formData: FormData) {
    setError(null);
    startTransition(async () => {
      const res = await renameUsername(formData);
      if (!res.ok) {
        setError(res.message ?? formatReason(res.reason));
      } else {
        setEditing(false);
      }
    });
  }

  if (!editing) {
    return (
      <div className="proto-username-row">
        <code className="proto-username-pill">@{currentUsername}</code>
        {decision.ok ? (
          <button
            type="button"
            className="proto-btn-link"
            onClick={() => {
              setValue(currentUsername);
              setEditing(true);
              setError(null);
            }}
          >
            Change ({MAX_SELF_RENAMES - renamesUsed} of {MAX_SELF_RENAMES} left)
          </button>
        ) : (
          <span className="proto-meta-quiet">{reasonLine(decision)}</span>
        )}
      </div>
    );
  }

  return (
    <form className="proto-form" action={onSubmit}>
      <label>
        New username
        <input
          type="text"
          name="username"
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
          }}
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          minLength={3}
          maxLength={24}
          required
          disabled={isPending}
          autoFocus
        />
        <span className="help">
          3–24 characters · letters, digits, and single dashes · must start
          and end with a letter or digit.
        </span>
      </label>
      {error ? <p className="proto-form-error" role="alert">{error}</p> : null}
      <div className="proto-form-actions">
        <button
          type="submit"
          className="proto-button-primary"
          disabled={
            isPending ||
            value.length < 3 ||
            value.trim().toLowerCase() === currentUsername.toLowerCase()
          }
        >
          {isPending ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          className="proto-btn-link"
          onClick={() => {
            setEditing(false);
            setValue(currentUsername);
            setError(null);
          }}
          disabled={isPending}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

function formatReason(reason: string): string {
  switch (reason) {
    case "validation":
      return "That username doesn't match the allowed shape.";
    case "reserved":
      return "That name is reserved.";
    case "taken":
      return "That name is already taken.";
    case "no-change":
      return "That's already your username.";
    case "grace_expired":
      return `Grace window ended after ${SELF_RENAME_GRACE_DAYS} days.`;
    case "count_exceeded":
      return `All ${MAX_SELF_RENAMES} changes used.`;
    case "cooldown":
      return `Wait ${SELF_RENAME_COOLDOWN_MINUTES} minutes between changes.`;
    case "unauth":
      return "Your session has ended. Sign in again.";
    default:
      return "Could not save that username.";
  }
}
