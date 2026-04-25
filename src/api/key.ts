// API key + OAuth token CRUD + copy/probe.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AccountUsage,
  ApiKeySummary,
  OauthTokenSummary,
  KeyCopyReceiptDto,
} from "../types";

export const keyApi = {
  // ---------- Keys (API keys + OAuth tokens) ----------
  /** List every stored ANTHROPIC_API_KEY — previews only, no secrets. */
  keyApiList: () => invoke<ApiKeySummary[]>("key_api_list"),
  /**
   * Add a new API key. Account is required — every key was created
   * under *some* account, and leaving that blank makes the row
   * un-findable by account later.
   */
  keyApiAdd: (label: string, token: string, accountUuid: string) =>
    invoke<ApiKeySummary>("key_api_add", { label, token, accountUuid }),
  keyApiRemove: (uuid: string) => invoke<void>("key_api_remove", { uuid }),
  /** Rename an API key. Label is user-owned metadata — no lookups
   *  key off it, so renames are display-only. */
  keyApiRename: (uuid: string, label: string) =>
    invoke<void>("key_api_rename", { uuid, label }),
  /**
   * Copy the full API key value to the OS clipboard. The raw secret
   * never returns to JS — Rust writes the clipboard directly and
   * schedules a 30-second self-clear. Returns a receipt the UI can
   * toast verbatim (label + preview + clear deadline).
   */
  keyApiCopy: (uuid: string) =>
    invoke<KeyCopyReceiptDto>("key_api_copy", { uuid }),
  /**
   * Validity ping against `GET /v1/models`. Resolves on a valid key;
   * rejects with a reason string ("rejected (invalid key)",
   * "rate-limited (retry in Ns)", …) that's safe to toast verbatim.
   * No DB write — result is transient.
   */
  keyApiProbe: (uuid: string) => invoke<void>("key_api_probe", { uuid }),

  /** List every stored CLAUDE_CODE_OAUTH_TOKEN — previews only. */
  keyOauthList: () => invoke<OauthTokenSummary[]>("key_oauth_list"),
  /**
   * Add a new OAuth token. Account tag is mandatory — the user picks
   * the account they ran `claude setup-token` against when created.
   */
  keyOauthAdd: (label: string, token: string, accountUuid: string) =>
    invoke<OauthTokenSummary>("key_oauth_add", {
      label,
      token,
      accountUuid,
    }),
  keyOauthRemove: (uuid: string) => invoke<void>("key_oauth_remove", { uuid }),
  /** Rename an OAuth token. See `keyApiRename`. */
  keyOauthRename: (uuid: string, label: string) =>
    invoke<void>("key_oauth_rename", { uuid, label }),
  /**
   * Sibling of `keyApiCopy` — same Rust-side clipboard write, same
   * receipt shape, same 30-second self-clear contract.
   */
  keyOauthCopy: (uuid: string) =>
    invoke<KeyCopyReceiptDto>("key_oauth_copy", { uuid }),
  /**
   * Copy a paste-ready POSIX shell invocation
   * (`CLAUDE_CODE_OAUTH_TOKEN='…' claude`) for this OAuth token. Same
   * receipt + clipboard contract as `keyOauthCopy` — the format string
   * is built on the Rust side so the secret never crosses the bridge.
   */
  keyOauthCopyShell: (uuid: string) =>
    invoke<KeyCopyReceiptDto>("key_oauth_copy_shell", { uuid }),
  /**
   * Cached usage snapshot for the account the OAuth token belongs to.
   * Never hits Anthropic — peeks the in-memory cache populated by
   * `fetchAllUsage` / `refreshUsageFor` on the Accounts side. Returns
   * `null` if no cached snapshot exists yet for that account.
   */
  keyOauthUsageCached: (uuid: string) =>
    invoke<AccountUsage | null>("key_oauth_usage_cached", { uuid }),

};
