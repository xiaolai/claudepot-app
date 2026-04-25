// Account / identity / desktop / app-status / usage DTOs.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.

/**
 * Keychain-free subset of `AccountSummary`, returned by
 * `api.accountListBasic()`. Use when a caller needs just the
 * sqlite-backed identity fields (uuid, email, org, subscription,
 * active flags) — it avoids the per-account Keychain reads the
 * full `AccountSummary` requires for `token_status` / `token_
 * remaining_mins` / `credentials_healthy`.
 */
export interface AccountSummaryBasic {
  uuid: string;
  email: string;
  org_name: string | null;
  subscription_type: string | null;
  is_cli_active: boolean;
  is_desktop_active: boolean;
  has_cli_credentials: boolean;
  has_desktop_profile: boolean;
}

export interface AccountSummary {
  uuid: string;
  email: string;
  org_name: string | null;
  subscription_type: string | null;
  is_cli_active: boolean;
  is_desktop_active: boolean;
  has_cli_credentials: boolean;
  has_desktop_profile: boolean;
  last_cli_switch: string | null; // RFC3339
  last_desktop_switch: string | null;
  token_status: string; // "valid (...)", "expired", "no credentials", ...
  token_remaining_mins: number | null;
  credentials_healthy: boolean; // true iff stored blob exists + parses
  /** "never" | "ok" | "drift" | "rejected" | "network_error" */
  verify_status: string;
  /** Server-observed email for this slot (may differ from `email` → drift). */
  verified_email: string | null;
  verified_at: string | null; // RFC3339
  /** True iff verified_email differs from email — misfiled slot. */
  drift: boolean;
  /**
   * Per-file-on-disk truth for the Desktop profile snapshot directory.
   * Differs from `has_desktop_profile` only when the DB flag has
   * drifted from disk. UI must prefer THIS field over `has_desktop_profile`
   * when gating Desktop affordances — the DB flag is a cached view.
   */
  desktop_profile_on_disk: boolean;
}

/**
 * Ground-truth "what is CC actually authenticated as" — the UI renders
 * this directly in the top-of-window truth strip. Equivalent of running
 * `claude auth status`.
 */
export interface CcIdentity {
  /** Email /api/oauth/profile returned, or null if CC has no blob. */
  email: string | null;
  /** RFC3339 timestamp of when we ran the profile check. */
  verified_at: string;
  /** Populated when CC has a blob but /profile failed. */
  error: string | null;
}

/**
 * How the Desktop identity was probed. Only `decrypted` is authoritative.
 * Callers that trigger mutation (Bind, switch, sign out) MUST require
 * `decrypted` — `org_uuid_candidate` is NOT verified.
 */
export type DesktopProbeMethod =
  | "org_uuid_candidate"
  | "decrypted"
  | "none";

/**
 * Ground-truth "who is Claude Desktop signed in as right now".
 * Mirrors `CcIdentity`: never throws at the Tauri boundary; all
 * failures ride `error` so banners can render them.
 *
 * Phase 1 only returns `org_uuid_candidate` or `none`; decrypted
 * path lands with Phase 2 crypto.
 */
export interface DesktopIdentity {
  email: string | null;
  org_uuid: string | null;
  probe_method: DesktopProbeMethod;
  verified_at: string; // RFC3339
  error: string | null;
}

export interface DesktopAdoptOutcome {
  account_email: string;
  captured_items: number;
  size_bytes: number;
}

export interface DesktopClearOutcome {
  email: string | null;
  snapshot_kept: boolean;
  items_deleted: number;
}

/**
 * Discriminated union matching `DesktopSyncOutcome` from Rust.
 * Serialized as `{ "kind": "verified", "email": "..." }` etc.
 */
export type DesktopSyncOutcome =
  | { kind: "no_live" }
  | { kind: "verified"; email: string }
  | { kind: "adoption_available"; email: string }
  | { kind: "stranger"; email: string }
  | { kind: "candidate_only"; email: string };

export interface AppStatus {
  platform: string; // "macos" | "linux" | "windows"
  arch: string;
  cli_active_email: string | null;
  desktop_active_email: string | null;
  desktop_installed: boolean;
  data_dir: string;
  /** Absolute path of `~/.claude`. Used to build session file paths
   * for Reveal-in-Finder without the webview guessing the home dir. */
  cc_config_dir: string;
  account_count: number;
}

export interface RegisterOutcome {
  email: string;
  org_name: string;
  subscription_type: string;
}

export interface RemoveOutcome {
  email: string;
  was_cli_active: boolean;
  was_desktop_active: boolean;
  had_desktop_profile: boolean;
  warnings: string[];
}

export interface UsageWindow {
  utilization: number; // 0–100
  /** RFC3339, or null when the window has no reset timestamp yet. */
  resets_at: string | null;
}

export interface ExtraUsage {
  is_enabled: boolean;
  /** Monthly cap in minor currency units (pence for GBP, cents for
   *  USD). Divide by 100 for display. */
  monthly_limit: number | null;
  /** Period spend in minor units (same basis as `monthly_limit`). */
  used_credits: number | null;
  /** Server-computed utilization percent — prefer over used/limit ratio. */
  utilization: number | null;
  /** ISO 4217 currency code ("USD", "GBP", …). Null on older
   *  responses; renderer falls back to USD. */
  currency: string | null;
}

export interface AccountUsage {
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  seven_day_opus: UsageWindow | null;
  seven_day_sonnet: UsageWindow | null;
  /** Third-party OAuth-app usage against this account (render-if-nonzero). */
  seven_day_oauth_apps: UsageWindow | null;
  /** Cowork / shared-seat usage pool (render-if-nonzero). */
  seven_day_cowork: UsageWindow | null;
  extra_usage: ExtraUsage | null;
}

/**
 * Per-account usage entry. Carries an explicit `status` so the UI can
 * render an inline explanation when data is unavailable, instead of
 * the old "silently omit the row" behavior.
 *
 * Status values:
 *   - "ok"              — fresh data (use `usage`)
 *   - "stale"           — cached data, see `age_secs` for staleness
 *   - "no_credentials"  — account has no blob (rare; filtered upstream)
 *   - "expired"         — token past local expiry → prompt re-login
 *   - "rate_limited"    — cooldown, see `retry_after_secs`
 *   - "error"           — other failure, see `error_detail`
 */
export interface UsageEntry {
  status:
    | "ok"
    | "stale"
    | "no_credentials"
    | "expired"
    | "rate_limited"
    | "error";
  usage: AccountUsage | null;
  age_secs: number | null;
  retry_after_secs: number | null;
  error_detail: string | null;
}

/** UUID string → usage entry. Every account with credentials appears
 *  here; the entry's `status` tells the UI whether to render data or
 *  an inline placeholder. */
export type UsageMap = Record<string, UsageEntry>;
