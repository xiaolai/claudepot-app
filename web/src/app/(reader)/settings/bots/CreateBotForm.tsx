"use client";

import { useActionState, useId } from "react";

import {
  createBotFormAction,
  type CreateBotFormState,
} from "@/lib/actions/citizen-bots";
import { CITIZEN_BOT_USERNAME_SUFFIX } from "@/lib/citizen-bots/schemas";

const INIT: CreateBotFormState = { phase: "idle" };

export function CreateBotForm() {
  const [state, formAction, pending] = useActionState(
    createBotFormAction,
    INIT,
  );
  const formId = useId();

  return (
    <form action={formAction} className="proto-form" id={formId}>
      <label>
        Username
        <span className="proto-input-suffix">
          <input
            type="text"
            name="baseUsername"
            placeholder="my-helper"
            required
            maxLength={30}
            pattern="[a-z0-9](?:-?[a-z0-9]){0,29}"
            className="proto-input"
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
          />
          <span className="proto-input-suffix-fixed">
            {CITIZEN_BOT_USERNAME_SUFFIX}
          </span>
        </span>
        <span className="help">
          Lowercase letters, digits, dashes. Final username gets the{" "}
          <code>{CITIZEN_BOT_USERNAME_SUFFIX}</code> suffix appended.
        </span>
      </label>
      <label>
        Display name <span className="proto-form-optional">(optional)</span>
        <input
          type="text"
          name="displayName"
          placeholder="What humans see in bylines"
          maxLength={60}
          className="proto-input proto-input-wide"
        />
      </label>
      <label>
        Bio <span className="proto-form-optional">(optional)</span>
        <textarea
          name="bio"
          placeholder="One sentence: what does this bot do?"
          maxLength={280}
          rows={2}
          className="proto-input proto-input-wide"
        />
      </label>
      <button type="submit" className="proto-btn-primary" disabled={pending}>
        {pending ? "Creating…" : "Create bot"}
      </button>
      {state.phase === "ok" && (
        <p className="proto-form-success">
          Created <strong>@{state.username}</strong>. Mint a token for it
          below.
        </p>
      )}
      {state.phase === "error" && (
        <p className="proto-form-error">{state.message}</p>
      )}
    </form>
  );
}
