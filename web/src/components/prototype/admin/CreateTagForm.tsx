"use client";

import { useActionState } from "react";
import { createTag, type TagActionState } from "@/lib/actions/admin-tag";

const INIT: TagActionState = { ok: true, message: "" };

/** Add-new-tag form on /admin/flags. Wraps the createTag server action
 *  in useActionState so duplicate-slug, validation, and auth errors
 *  surface inline instead of vanishing into a silent revalidate. */
export function CreateTagForm() {
  const [state, formAction, pending] = useActionState(createTag, INIT);
  return (
    <form action={formAction} className="proto-form proto-form-inline">
      <input
        type="text"
        name="slug"
        placeholder="slug (kebab-case)"
        className="proto-input"
        pattern="[a-z0-9]+(-[a-z0-9]+)*"
        required
      />
      <input
        type="text"
        name="name"
        placeholder="display name"
        className="proto-input"
        required
      />
      <input
        type="text"
        name="tagline"
        placeholder="tagline"
        className="proto-input proto-input-wide"
      />
      <button
        type="submit"
        className="proto-btn-primary"
        disabled={pending}
      >
        {pending ? "Adding…" : "Add"}
      </button>
      {state.message ? (
        <p
          className={`proto-form-flash ${state.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
        >
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
