"use client";

import { useActionState, useId, useState } from "react";

import {
  mintApiTokenFormAction,
  type MintFormState,
} from "@/lib/actions/api-tokens";
import {
  PRIVILEGED_SCOPES,
  SCOPE_GROUPS,
  SCOPE_LABELS,
  type Scope,
} from "@/lib/api/scopes";

const INIT: MintFormState = { phase: "idle" };

/**
 * Mint flow:
 *
 *   submit form → server action → on success the action sets a
 *   one-time encrypted cookie (see `lib/api/reveal-cookie.ts`) and
 *   redirects to `/settings/tokens/reveal`. The plaintext never
 *   crosses the useActionState boundary, so it never enters the
 *   React client heap inside this component.
 *
 *   On failure the action returns an error in the form state, which
 *   we render below the form.
 */
export function MintTokenForm({ staff }: { staff: boolean }) {
  const [state, formAction, pending] = useActionState(
    mintApiTokenFormAction,
    INIT,
  );
  const [selected, setSelected] = useState<Set<Scope>>(new Set());
  const formId = useId();

  // Privileged scopes (Editorial + Bots groups) are staff/bot-only.
  // Session-holding minters are humans, so `staff` is the UI-side
  // entitlement signal; createApiToken re-checks server-side against
  // the DB, so hiding here is presentation, not the security gate.
  const visibleGroups = SCOPE_GROUPS.map((group) => ({
    label: group.label,
    scopes: staff
      ? group.scopes
      : group.scopes.filter((s) => !PRIVILEGED_SCOPES.has(s)),
  })).filter((group) => group.scopes.length > 0);
  const visibleScopes = visibleGroups.flatMap((group) => group.scopes);

  const allSelected = selected.size === visibleScopes.length;
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
    setSelected(on ? new Set(visibleScopes) : new Set());
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
        {visibleGroups.map((group) => (
          <div key={group.label} className="proto-token-scope-group">
            <p className="proto-token-scope-group-label">{group.label}</p>
            {group.scopes.map((s) => (
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
          </div>
        ))}
        <span className="help">
          Pick the smallest set of scopes the token actually needs. Most
          read-only bots want just <code>read:all</code>. You can revoke
          and re-mint with different scopes anytime.
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
