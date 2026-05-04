"use client";

import { useActionState } from "react";
import {
  mergeTag,
  renameTag,
  retireTag,
  type TagActionState,
} from "@/lib/actions/admin-tag";

const INIT: TagActionState = { ok: true, message: "" };

/** One row of the /admin/flags table. Owns three independent action
 *  states (rename / merge / retire) so each form's success/error
 *  message is local to its form; clicking Save doesn't clear an
 *  in-flight Merge flash, etc.
 *
 *  HTML5 `form="rename-<slug>"` lets the tagline input live in the
 *  next cell while still submitting through the rename form — single
 *  Save button posts both fields. */
export function TagRow({
  slug,
  name,
  tagline,
  posts,
}: {
  slug: string;
  name: string;
  tagline: string | null;
  posts: number;
}) {
  const [renameState, renameAction, renamePending] = useActionState(
    renameTag,
    INIT,
  );
  const [mergeState, mergeAction, mergePending] = useActionState(
    mergeTag,
    INIT,
  );
  const [retireState, retireAction, retirePending] = useActionState(
    retireTag,
    INIT,
  );

  const formId = `rename-${slug}`;

  return (
    <tr>
      <td>
        <code>{slug}</code>
      </td>
      <td>
        <form id={formId} action={renameAction} className="proto-tag-edit">
          <input type="hidden" name="slug" value={slug} />
          <input
            type="text"
            name="name"
            defaultValue={name}
            className="proto-input proto-input-inline"
            aria-label={`Display name for ${slug}`}
            required
          />
          <button
            type="submit"
            className="proto-mod-btn proto-mod-btn-keep"
            disabled={renamePending}
          >
            {renamePending ? "Saving…" : "Save"}
          </button>
        </form>
        {renameState.message ? (
          <p
            className={`proto-form-flash ${renameState.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
          >
            {renameState.message}
          </p>
        ) : null}
      </td>
      <td>
        <input
          type="text"
          name="tagline"
          form={formId}
          defaultValue={tagline ?? ""}
          className="proto-input proto-input-inline proto-input-wide"
          aria-label={`Tagline for ${slug}`}
        />
      </td>
      <td>{posts}</td>
      <td className="proto-mod-actions">
        <form action={mergeAction} className="proto-tag-merge">
          <input type="hidden" name="fromSlug" value={slug} />
          <input
            type="text"
            name="toSlug"
            placeholder="merge into…"
            className="proto-input proto-input-inline"
            aria-label={`Merge ${slug} into…`}
            pattern="[a-z0-9]+(-[a-z0-9]+)*"
            required
          />
          <button
            type="submit"
            className="proto-mod-btn proto-mod-btn-warn"
            disabled={mergePending}
          >
            {mergePending ? "Merging…" : "Merge"}
          </button>
        </form>
        <form action={retireAction}>
          <input type="hidden" name="slug" value={slug} />
          <button
            type="submit"
            className="proto-mod-btn proto-mod-btn-remove"
            disabled={retirePending}
          >
            {retirePending ? "Retiring…" : "Retire"}
          </button>
        </form>
        {mergeState.message ? (
          <p
            className={`proto-form-flash ${mergeState.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
          >
            {mergeState.message}
          </p>
        ) : null}
        {retireState.message ? (
          <p
            className={`proto-form-flash ${retireState.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
          >
            {retireState.message}
          </p>
        ) : null}
      </td>
    </tr>
  );
}
