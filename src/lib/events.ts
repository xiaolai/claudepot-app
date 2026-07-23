/**
 * Frontend mirror of the Tauri event-channel names the webview both
 * EMITS and LISTENS for. The VALUES are a wire contract shared with the
 * Rust backend — they must match `src-tauri/src/events.rs` byte for
 * byte, since the renderer subscribes/emits by exact string. Centralize
 * a name here once more than one FE site references it, so a typo can't
 * silently break the cross-boundary contract. (Single-site event names
 * stay as inline literals at their one call site, per the existing
 * convention — this file is for the shared ones.)
 */

/** Emitted whenever an account's credentials are healed — by the
 *  background token-refresh orchestrator or a UI-driven verify — so the
 *  Accounts screen re-pulls usage instead of leaving a stale "token
 *  expired" placeholder. Emitters: `runVerifyAll`, `useAccountHandlers`,
 *  and the Rust `token_refresh_orchestrator`; listener: `useUsage`.
 *  Mirrors `src-tauri/src/events.rs::USAGE_REFETCH`. */
export const USAGE_REFETCH_EVENT = "usage::refetch";
