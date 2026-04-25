// API key + OAuth token DTOs.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.


// ---------- Keys (API keys + OAuth tokens) ----------

/**
 * One `ANTHROPIC_API_KEY` row in the Keys section. Secret itself never
 * leaves the Rust side — only the truncated `token_preview` (e.g.
 * `sk-ant-api03-Abc…xyz`) is safe to render. Call `keyApiCopy` to
 * deliberately pull the full value into the clipboard.
 */
export interface ApiKeySummary {
  uuid: string;
  label: string;
  token_preview: string;
  account_uuid: string;
  /** Email joined from `accounts.db` at read time. Null only when the
   *  linked account has been removed (orphan state). */
  account_email: string | null;
  created_at: string; // RFC3339
  last_probed_at: string | null;
  last_probe_status: string | null;
}

/**
 * Receipt returned by `keyApiCopy` / `keyOauthCopy` /
 * `keyOauthCopyShell`. The raw secret is written to the OS clipboard
 * by Rust and never crosses the IPC bridge; the renderer only sees
 * fields it already had on hand (label + preview) plus the unix-ms
 * timestamp at which Rust will self-clear the clipboard. Designed
 * to be safe to log + toast verbatim.
 */
export interface KeyCopyReceiptDto {
  label: string;
  preview: string;
  clipboard_clears_at_unix_ms: number;
}

/**
 * One `CLAUDE_CODE_OAUTH_TOKEN` row. Account tag is mandatory —
 * the user picks the account they ran `claude setup-token` against
 * when they add the token. `expires_at` is a 365-day proxy off
 * `created_at`; the authoritative signal is `last_probe_status ===
 * "unauthorized"`, which comes back from `/api/oauth/usage`.
 */
export interface OauthTokenSummary {
  uuid: string;
  label: string;
  token_preview: string;
  account_uuid: string;
  account_email: string | null;
  created_at: string;
  expires_at: string;
  days_remaining: number;
  last_probed_at: string | null;
  last_probe_status: string | null;
}
