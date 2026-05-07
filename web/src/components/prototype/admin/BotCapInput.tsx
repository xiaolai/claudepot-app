"use client";

import { useActionState, useId } from "react";

import {
  setBotMonthlyCapFormAction,
  type SetCapActionState,
} from "@/lib/actions/admin-bot";

const INIT: SetCapActionState = { ok: true, message: "" };

/**
 * Inline cap-edit cell for /admin/console/users. Renders only on
 * is_agent=true rows; non-agent users have no cap (the column is
 * irrelevant and the server action would reject anyway).
 *
 * Empty input on submit clears the cap. The current value is the
 * input's defaultValue; staff edits in place and presses Save.
 * useActionState surfaces success/error as a small flash next to
 * the field.
 */
export function BotCapInput({
  userId,
  current,
}: {
  userId: string;
  /** Current cap as a string from numeric() — null = no cap. */
  current: string | null;
}) {
  const [state, formAction, pending] = useActionState(
    setBotMonthlyCapFormAction,
    INIT,
  );
  const inputId = useId();

  const display = current === null ? "" : Number.parseFloat(current).toFixed(2);

  return (
    <form action={formAction} className="proto-cap-input">
      <input type="hidden" name="userId" value={userId} />
      <label htmlFor={inputId} className="proto-sr-only">
        Monthly USD cap
      </label>
      <span aria-hidden>$</span>
      <input
        id={inputId}
        type="number"
        name="cap"
        defaultValue={display}
        step="0.01"
        min="0"
        max="1000000"
        placeholder="—"
        className="proto-input proto-input-narrow"
        title={
          current === null
            ? "No cap. Enter a USD value and press Save."
            : `Current cap: $${display}. Empty to clear.`
        }
      />
      <button
        type="submit"
        className="proto-mod-btn proto-mod-btn-keep"
        disabled={pending}
      >
        {pending ? "…" : "Save"}
      </button>
      {state.message ? (
        <span
          className={
            state.ok
              ? "proto-form-flash proto-form-flash-ok"
              : "proto-form-flash proto-form-flash-err"
          }
        >
          {state.message}
        </span>
      ) : null}
    </form>
  );
}
