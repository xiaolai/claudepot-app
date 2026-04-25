import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
import type {
  AccountSummary,
  OperationProgressEvent,
  VerifyAccountEvent,
  VerifyOutcomeKind,
} from "../../types";

/**
 * Run a `verify_all` op end-to-end via the streaming `*_start` IPC.
 *
 * Subscribes to `op-progress::<op_id>` for the duration. As each
 * `VerifyAccountEvent` lands, the corresponding row is patched in place
 * via `patchAccount`, so the UI flips per-row badges in real time
 * instead of waiting for the whole loop to finish before re-rendering.
 *
 * On the terminal `op` event, we make ONE final `accountList()` call
 * to pick up the persisted `verified_email` / `verified_at` / freshly
 * recomputed token_health columns — those are computed in the DTO
 * layer and aren't streamed per-row.
 *
 * Resolves when the terminal event lands (or rejects if the start call
 * itself failed). Per-account errors flow through the sink as
 * `network_error` outcomes; this function does not throw on them.
 */
export interface RunVerifyAllOptions {
  /** Patch a single account row in place. The caller decides where the
   *  row lives (state, store, etc.) — this just passes the new shape. */
  patchAccount: (uuid: string, patch: Partial<AccountSummary>) => void;
  /** Replace the full list once the op terminates, after re-fetching. */
  setAccounts: (rows: AccountSummary[]) => void;
}

export interface VerifyAllOutcome {
  total: number;
  ok: number;
  drift: number;
  rejected: number;
  network_error: number;
}

export async function runVerifyAll(
  opts: RunVerifyAllOptions,
): Promise<VerifyAllOutcome> {
  const opId = await api.verifyAllAccountsStart();
  const channel = `op-progress::${opId}`;

  // Tally per-account outcomes locally so callers don't have to re-derive
  // from a shape mismatch between the typed sub-events and the polling
  // backstop.
  const counts: VerifyAllOutcome = {
    total: 0,
    ok: 0,
    drift: 0,
    rejected: 0,
    network_error: 0,
  };

  return new Promise<VerifyAllOutcome>((resolve, reject) => {
    let unlistenFn: (() => void) | null = null;
    let settled = false;

    const finalize = async (terminalErr: string | null) => {
      if (settled) return;
      settled = true;
      if (unlistenFn) {
        try {
          unlistenFn();
        } catch {
          /* ignore */
        }
      }
      // Always pull the fresh DB state so verified_email / token_health
      // / verify_status columns get into the UI even if a per-row event
      // dropped en route. Doing this even on terminal-error is safe:
      // the per-account errors already flowed through the sink.
      try {
        const refreshed = await api.accountList();
        opts.setAccounts(refreshed);
      } catch {
        // Non-fatal; the caller's next focus/refresh tick picks it up.
      }
      if (terminalErr) {
        reject(new Error(terminalErr));
      } else {
        resolve(counts);
      }
    };

    const handler = (event: {
      payload: OperationProgressEvent | VerifyAccountEvent;
    }) => {
      const ev = event.payload;
      if (ev.op_id !== opId) return;
      // Discriminate the typed `VerifyAccountEvent` from the generic
      // `OperationProgressEvent` by the `kind` field. The generic
      // event has no `kind`; the verify event sets `"verify_account"`.
      if ("kind" in ev && ev.kind === "verify_account") {
        const row = ev as VerifyAccountEvent;
        counts.total = row.total;
        counts[row.outcome as VerifyOutcomeKind] += 1;
        opts.patchAccount(row.uuid, {
          verify_status: row.outcome,
          // Surface the actual_email when drift is reported. The DB row
          // gets the same write on the backend; this just keeps the UI
          // in sync until the post-Done accountList lands.
          ...(row.outcome === "drift" && row.detail
            ? { verified_email: extractActualEmail(row.detail) }
            : {}),
        });
        return;
      }
      // Generic phase / terminal event.
      const generic = ev as OperationProgressEvent;
      if (generic.phase === "op") {
        const detail =
          generic.status === "error" ? generic.detail ?? "verify failed" : null;
        void finalize(detail);
      }
    };

    listen<OperationProgressEvent | VerifyAccountEvent>(channel, handler)
      .then((fn) => {
        unlistenFn = fn;
      })
      .catch((err) => {
        // listen failed — surface as a terminal error so the caller can
        // decrement its busy counter.
        void finalize(`subscribe failed: ${err}`);
      });
  });
}

/** Drift detail format from core: `"actual: foo@bar.com"`. Extract the
 *  email; returns null if the format ever changes so the patch becomes
 *  a no-op rather than writing garbage. */
function extractActualEmail(detail: string): string | null {
  const m = detail.match(/^actual:\s*(.+)$/);
  return m ? m[1].trim() : null;
}
