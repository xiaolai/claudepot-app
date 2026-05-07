"use client";

import type { ReactNode } from "react";
import { useActionState } from "react";

import {
  proposalActionForm,
  type ProposalActionState,
} from "@/lib/actions/admin-proposal";

const INIT: ProposalActionState = { ok: true, message: "" };

interface Props {
  reportId: string;
  action: "accept" | "reject";
  className?: string;
  pendingLabel?: string;
  children: ReactNode;
}

/**
 * Inline form for accepting / rejecting a bot proposal. Mirrors the
 * ModButton pattern — single-button form, useActionState surfaces
 * validation errors inline, success flash is silent because
 * revalidatePath in the action will swap the row on the next render.
 */
export function ProposalActionButton({
  reportId,
  action,
  className,
  pendingLabel = "Working…",
  children,
}: Props) {
  const [state, formAction, pending] = useActionState(
    proposalActionForm,
    INIT,
  );
  return (
    <form action={formAction} className="proto-inline-form">
      <input type="hidden" name="reportId" value={reportId} />
      <input type="hidden" name="action" value={action} />
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
