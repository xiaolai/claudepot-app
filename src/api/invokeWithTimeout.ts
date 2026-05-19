// Timeout-bounded wrapper around Tauri's `invoke` for IPC calls whose
// Rust side touches the macOS keychain (directly or transitively
// through `swap::load_private` / `save_private` / `delete_private`).
//
// Why this exists. Each `/usr/bin/security` subprocess call is bounded
// at 5 s on the Rust side (see `cli_backend::storage::KEYCHAIN_TIMEOUT`)
// with `kill_on_drop(true)`. For multi-account calls — `account_list`,
// `verify_all_accounts`, `accounts_reconcile`, `fetch_all_usage` —
// the worst-case latency is `5s × N`. Without a JS-side ceiling the
// renderer's Promise sits unresolved for the full window, and any
// caller that awaits unconditionally (`useAccounts`, `useRefresh`,
// `runVerifyAll`) makes the Accounts pane appear frozen while every
// other surface keeps moving.
//
// The Promise.race here is the JS-side ceiling that surfaces a
// "keychain probe stalled" error instead of an indefinite spin. The
// underlying IPC keeps running until the Rust-side timeout fires —
// we don't cancel it (Tauri 2's `invoke` has no AbortSignal hook in
// stable as of 2.x), but the caller's await unblocks and the UI can
// render a Retry affordance.
//
// Do NOT use this wrapper for IPC calls that intentionally wait on the
// user (`account_login`, `account_register_from_browser`) — those use
// the `*_start` + `op-progress::<op_id>` pattern and return the op_id
// in <100ms, so they don't need a ceiling.

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

/**
 * Error thrown when an `invokeWithTimeout` call exceeds its `ms`
 * budget. Distinguishable from native IPC errors via `instanceof`
 * so the UI can surface a "Retry" affordance instead of conflating
 * it with a real Rust-side failure.
 */
export class IpcTimeoutError extends Error {
  readonly command: string;
  readonly ms: number;
  constructor(command: string, ms: number) {
    super(`IPC \`${command}\` exceeded ${ms}ms — keychain probe stalled?`);
    this.name = "IpcTimeoutError";
    this.command = command;
    this.ms = ms;
  }
}

/**
 * Race a Tauri IPC call against a millisecond budget. Resolves with
 * the IPC result on success; rejects with `IpcTimeoutError` on
 * timeout or whatever the IPC itself rejected with on a Rust-side
 * error.
 *
 * The clearTimeout in the finally branch is load-bearing — without
 * it the timeout handle leaks for `ms` milliseconds after success
 * (no functional bug, but it shows up as "lingering timer" in
 * profilers and devtools).
 */
export async function invokeWithTimeout<T>(
  command: string,
  args: InvokeArgs | undefined,
  ms: number,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new IpcTimeoutError(command, ms)), ms);
  });
  try {
    return await Promise.race([invoke<T>(command, args), timeoutPromise]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}
