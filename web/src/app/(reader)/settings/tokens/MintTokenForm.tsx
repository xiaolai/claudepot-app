"use client";

import { useActionState, useId, useState } from "react";

import {
  mintApiTokenFormAction,
  type MintFormState,
} from "@/lib/actions/api-tokens";
import { SCOPES, SCOPE_LABELS, type Scope } from "@/lib/api/scopes";

const INIT: MintFormState = { phase: "idle" };

export function MintTokenForm({ staff }: { staff: boolean }) {
  const [state, formAction, pending] = useActionState(
    mintApiTokenFormAction,
    INIT,
  );
  const [copied, setCopied] = useState(false);
  const [selected, setSelected] = useState<Set<Scope>>(new Set());
  const formId = useId();

  const allSelected = selected.size === SCOPES.length;
  const someSelected = selected.size > 0 && !allSelected;

  function toggleScope(s: Scope, on: boolean) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(s);
      else next.delete(s);
      return next;
    });
  }

  function toggleAll(on: boolean) {
    setSelected(on ? new Set(SCOPES) : new Set());
  }

  async function copyPlaintext(plaintext: string) {
    try {
      await navigator.clipboard.writeText(plaintext);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  }

  if (state.phase === "ok") {
    return (
      <div className="proto-token-flash">
        <p className="proto-form-flash proto-form-flash-ok">
          Token <strong>{state.tokenName}</strong> ({state.displayPrefix}…)
          minted. Copy it now — it cannot be shown again.
        </p>
        <code className="proto-token-plaintext">{state.plaintext}</code>
        <div className="proto-form-inline">
          <button
            type="button"
            className="proto-btn-primary"
            onClick={() => copyPlaintext(state.plaintext)}
          >
            {copied ? "Copied" : "Copy"}
          </button>
          <a href="/settings/tokens" className="proto-btn-secondary">
            Done
          </a>
        </div>
      </div>
    );
  }

  return (
    <form action={formAction} className="proto-form" id={formId}>
      <label>
        Name
        <input
          type="text"
          name="name"
          placeholder="e.g. ada-runner-mini"
          className="proto-input proto-input-wide"
          required
          maxLength={80}
        />
        <span className="help">
          A label for your own reference. Shown only to you.
        </span>
      </label>

      <fieldset className="proto-form-fieldset">
        <legend>Scopes</legend>
        <label className="proto-token-scope-toggle-all">
          <input
            type="checkbox"
            checked={allSelected}
            ref={(el) => {
              if (el) el.indeterminate = someSelected;
            }}
            onChange={(e) => toggleAll(e.currentTarget.checked)}
          />
          <strong>Select all</strong>
        </label>
        {SCOPES.map((s) => (
          <label key={s}>
            <input
              type="checkbox"
              name="scopes"
              value={s}
              checked={selected.has(s)}
              onChange={(e) => toggleScope(s, e.currentTarget.checked)}
            />
            <code>{s}</code> — {SCOPE_LABELS[s]}
          </label>
        ))}
        <span className="help">
          Pick the smallest set of scopes the token actually needs. You can
          revoke and re-mint with different scopes anytime.
        </span>
      </fieldset>

      <label>
        Expires after
        <select
          name="expiresInDays"
          className="proto-input"
          defaultValue="180"
        >
          <option value="30">30 days</option>
          <option value="90">90 days</option>
          <option value="180">180 days (default)</option>
          <option value="365">365 days</option>
          {staff ? <option value="never">never (staff only)</option> : null}
        </select>
      </label>

      <button
        type="submit"
        className="proto-btn-primary"
        disabled={pending}
      >
        {pending ? "Minting…" : "Mint token"}
      </button>

      {state.phase === "err" ? (
        <p className="proto-form-flash proto-form-flash-err">{state.message}</p>
      ) : null}
    </form>
  );
}
