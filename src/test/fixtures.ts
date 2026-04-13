// Fixtures for React component tests. The Tauri `invoke` mock itself lives in
// App.test.tsx (via vi.doMock before dynamic import), so tests can provide
// per-command handlers without global state.

import type { AccountSummary, AppStatus } from "../types";

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
  ...overrides,
});
