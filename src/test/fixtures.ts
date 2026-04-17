// Fixtures for React component tests. The Tauri `invoke` mock itself lives in
// App.test.tsx (via vi.doMock before dynamic import), so tests can provide
// per-command handlers without global state.

import type { AccountSummary, AppStatus, UsageEntry } from "../types";

export const sampleStatus = (overrides?: Partial<AppStatus>): AppStatus => ({
  platform: "macos",
  arch: "aarch64",
  cli_active_email: null,
  desktop_active_email: null,
  desktop_installed: true,
  data_dir: "/tmp/claudepot-test",
  account_count: 0,
  ...overrides,
});

export const sampleAccount = (
  overrides?: Partial<AccountSummary>,
): AccountSummary => ({
  uuid: "aaaa1111-2222-4333-8444-555555555555",
  email: "alice@example.com",
  org_name: "Alice Org",
  subscription_type: "max",
  is_cli_active: false,
  is_desktop_active: false,
  has_cli_credentials: true,
  has_desktop_profile: false,
  last_cli_switch: null,
  last_desktop_switch: null,
  token_status: "valid (47m remaining)",
  token_remaining_mins: 47,
  credentials_healthy: true,
  verify_status: "ok",
  verified_email: "alice@example.com",
  verified_at: null,
  drift: false,
  ...overrides,
});

/** Factory for UsageEntry fixtures. Every test that renders a usage
 *  row can pick a status and supply overrides for the rest. */
export const sampleUsageEntry = (
  overrides?: Partial<UsageEntry>,
): UsageEntry => ({
  status: "ok",
  usage: {
    five_hour: { utilization: 42, resets_at: null },
    seven_day: null,
    seven_day_opus: null,
    seven_day_sonnet: null,
    extra_usage: null,
  },
  age_secs: 5,
  retry_after_secs: null,
  error_detail: null,
  ...overrides,
});
