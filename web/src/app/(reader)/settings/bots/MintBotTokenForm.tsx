"use client";

import { useActionState, useId, useState } from "react";

import {
  mintBotTokenFormAction,
  type MintBotTokenFormState,
} from "@/lib/actions/citizen-bots";
import { CITIZEN_SCOPES } from "@/lib/citizen-bots/scopes";
import { SCOPE_LABELS } from "@/lib/api/scopes";

const INIT: MintBotTokenFormState = { phase: "idle" };

type Props = {
  botId: string;
  username: string;
};

export function MintBotTokenForm({ botId, username }: Props) {
  const [state, formAction, pending] = useActionState(
    mintBotTokenFormAction,
    INIT,
  );
  const formId = useId();
  const [copied, setCopied] = useState(false);

  // Pre-select all citizen-allowed scopes; users can untick.
  const [selected, setSelected] = useState<Set<string>>(
    new Set<string>(CITIZEN_SCOPES),
  );

  function toggle(scope: string, on: boolean) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(scope);
      else next.delete(scope);
      return next;
    });
  }

  async function copyPlaintext(plaintext: string) {
    try {
      await navigator.clipboard.writeText(plaintext);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Browsers without clipboard API fall back to manual select.
    }
  }

  return (
    <form action={formAction} className="proto-form" id={formId}>
      <input type="hidden" name="botId" value={botId} />
      <label>
        Token name
        <input
          type="text"
          name="name"
          placeholder={`${username}-runner`}
          required
          maxLength={80}
          className="proto-input proto-input-wide"
        />
        <span className="help">Your label for this token. Shown only to you.</span>
      </label>
      <fieldset className="proto-form-fieldset">
        <legend>Scopes</legend>
        {CITIZEN_SCOPES.map((s) => (
          <label key={s} className="proto-token-scope-row">
            <input
              type="checkbox"
              name="scopes"
              value={s}
              checked={selected.has(s)}
              onChange={(e) => toggle(s, e.currentTarget.checked)}
            />
            <code>{s}</code>
            <span className="proto-token-scope-label">{SCOPE_LABELS[s] ?? s}</span>
          </label>
        ))}
        <p className="help">
          Citizen bots are limited to this allowlist. Other scopes are
          rejected at mint time.
        </p>
      </fieldset>
      <button type="submit" className="proto-btn-primary" disabled={pending}>
        {pending ? "Minting…" : "Mint token"}
      </button>
      {state.phase === "ok" && (
        <div className="proto-token-reveal">
          <p className="proto-form-success">
            <strong>Save this token now.</strong> It is shown once. After
            you leave this page it cannot be retrieved.
          </p>
          <pre className="proto-token-plaintext" aria-label="New token">
            {state.plaintext}
          </pre>
          <button
            type="button"
            className="proto-btn-secondary"
            onClick={() => copyPlaintext(state.plaintext)}
          >
            {copied ? "Copied ✓" : "Copy"}
          </button>
          <p className="proto-form-hint">
            Granted scopes:{" "}
            {state.granted.map((s) => (
              <code key={s} style={{ marginRight: "var(--sp-6)" }}>
                {s}
              </code>
            ))}
          </p>
          {state.dropped.length > 0 && (
            <p className="proto-form-hint" style={{ color: "var(--state-rejected-fg)" }}>
              Dropped (not allowed for citizen bots):{" "}
              {state.dropped.map((s) => (
                <code key={s} style={{ marginRight: "var(--sp-6)" }}>
                  {s}
                </code>
              ))}
            </p>
          )}
        </div>
      )}
      {state.phase === "error" && (
        <p className="proto-form-error">{state.message}</p>
      )}
    </form>
  );
}
