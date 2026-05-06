"use client";

import { useActionState } from "react";

import {
  botExemptFormAction,
  type BotExemptActionState,
} from "@/lib/actions/admin-bot";

const INIT: BotExemptActionState = { ok: true, message: "" };

/**
 * One-click toggle for users.bot_moderation_exempt.
 *
 * Renders only on rows where is_agent=true; non-agent users have no
 * toggle at all (the column is irrelevant for them, and the server
 * action would reject anyway).
 */
export function BotExemptToggle({
  userId,
  current,
}: {
  userId: string;
  current: boolean;
}) {
  const [state, formAction, pending] = useActionState(
    botExemptFormAction,
    INIT,
  );

  // The form submits the inverse of `current` so the click toggles.
  const next = !current;

  return (
    <form action={formAction}>
      <input type="hidden" name="userId" value={userId} />
      <input type="hidden" name="exempt" value={next ? "true" : "false"} />
      <button
        type="submit"
        className={
          current
            ? "proto-mod-btn proto-mod-btn-keep"
            : "proto-mod-btn proto-mod-btn-remove"
        }
        disabled={pending}
        title={
          current
            ? "Click to re-enable AI policy moderation for this bot"
            : "Click to skip the AI policy moderator for this bot"
        }
      >
        {pending
          ? current
            ? "Re-enabling…"
            : "Exempting…"
          : current
            ? "Exempt"
            : "Moderated"}
      </button>
      {state.message && !state.ok ? (
        <span className="proto-form-flash proto-form-flash-err">
          {state.message}
        </span>
      ) : null}
    </form>
  );
}
