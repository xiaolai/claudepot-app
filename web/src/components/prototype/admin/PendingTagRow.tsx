"use client";

import { useActionState } from "react";
import {
  approvePendingTag,
  rejectPendingTag,
  type TagActionState,
} from "@/lib/actions/admin-tag";

const INIT: TagActionState = { ok: true, message: "" };

/**
 * One row of the /admin/flags pending-review table. Lists an
 * Ada-proposed tag with sample submissions linking it.
 *
 * Approve flips pending_review=false and (optionally) edits the
 * placeholder name. Reject deletes the tag (cascading
 * submission_tags rows) — staff has decided it shouldn't enter the
 * vocabulary.
 *
 * Two independent action states so each form's flash message is
 * scoped to its own submit button.
 */
export function PendingTagRow({
  slug,
  name,
  tagline,
  postCount,
  sampleTitles,
}: {
  slug: string;
  name: string;
  tagline: string | null;
  postCount: number;
  sampleTitles: string[];
}) {
  const [approveState, approveAction, approvePending] = useActionState(
    approvePendingTag,
    INIT,
  );
  const [rejectState, rejectAction, rejectPending] = useActionState(
    rejectPendingTag,
    INIT,
  );

  const approveFormId = `approve-${slug}`;

  return (
    <tr>
      <td>
        <code>{slug}</code>
      </td>
      <td>
        <form
          id={approveFormId}
          action={approveAction}
          className="proto-tag-edit"
        >
          <input type="hidden" name="slug" value={slug} />
          <input
            type="text"
            name="name"
            defaultValue={name}
            className="proto-input proto-input-inline"
            aria-label={`Display name for ${slug}`}
            required
          />
        </form>
        {approveState.message ? (
          <p
            className={`proto-form-flash ${approveState.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
          >
            {approveState.message}
          </p>
        ) : null}
      </td>
      <td>
        <input
          type="text"
          name="tagline"
          form={approveFormId}
          defaultValue={tagline ?? ""}
          className="proto-input proto-input-inline proto-input-wide"
          aria-label={`Tagline for ${slug}`}
          placeholder="Optional one-line description"
        />
      </td>
      <td>
        {postCount}
        {sampleTitles.length > 0 ? (
          <ul className="proto-pending-samples">
            {sampleTitles.map((title) => (
              <li key={title} title={title}>
                {title}
              </li>
            ))}
          </ul>
        ) : null}
      </td>
      <td className="proto-mod-actions">
        <button
          type="submit"
          form={approveFormId}
          className="proto-mod-btn proto-mod-btn-keep"
          disabled={approvePending}
        >
          {approvePending ? "Approving…" : "Approve"}
        </button>
        <form action={rejectAction}>
          <input type="hidden" name="slug" value={slug} />
          <button
            type="submit"
            className="proto-mod-btn proto-mod-btn-remove"
            disabled={rejectPending}
          >
            {rejectPending ? "Rejecting…" : "Reject"}
          </button>
        </form>
        {rejectState.message ? (
          <p
            className={`proto-form-flash ${rejectState.ok ? "proto-form-flash-ok" : "proto-form-flash-err"}`}
          >
            {rejectState.message}
          </p>
        ) : null}
      </td>
    </tr>
  );
}
