"use client";

import type { ReactNode } from "react";
import { useActionState } from "react";
import {
  moderationFormAction,
  type ModActionState,
} from "@/lib/actions/moderation";

const INIT: ModActionState = { ok: true, message: "" };

type ModButtonAction =
  | "approve"
  | "reject"
  | "dismiss_flag"
  | "delete"
  | "lock_user"
  | "lock"
  | "unlist"
  | "restore";

/** Single-button form for /admin/queue and /admin/users.
 *
 *  Wraps moderationFormAction in useActionState so role-check denials
 *  and validation errors surface inline rather than being absorbed
 *  silently. Successful actions render a quiet success flash;
 *  revalidatePath in the underlying moderationAction re-fetches the
 *  server data so the row will swap state on the next render anyway.
 *
 *  pendingLabel defaults to "Working…" — pass an action-specific verb
 *  for clearer feedback ("Suspending…", "Approving…"). */
export function ModButton({
  action,
  targetType,
  targetId,
  flagId,
  className,
  pendingLabel = "Working…",
  children,
}: {
  action: ModButtonAction;
  targetType?: "submission" | "comment";
  targetId: string;
  flagId?: string;
  className?: string;
  pendingLabel?: string;
  children: ReactNode;
}) {
  const [state, formAction, pending] = useActionState(
    moderationFormAction,
    INIT,
  );
  return (
    <form action={formAction}>
      <input type="hidden" name="action" value={action} />
      {targetType ? (
        <input type="hidden" name="targetType" value={targetType} />
      ) : null}
      <input type="hidden" name="targetId" value={targetId} />
      {flagId ? <input type="hidden" name="flagId" value={flagId} /> : null}
      <button type="submit" className={className} disabled={pending}>
        {pending ? pendingLabel : children}
      </button>
      {state.message && !state.ok ? (
        <span className="proto-form-flash proto-form-flash-err">
          {state.message}
        </span>
      ) : null}
    </form>
  );
}
