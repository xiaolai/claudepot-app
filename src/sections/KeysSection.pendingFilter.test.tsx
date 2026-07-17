/**
 * KeysSection — cross-section pending-filter handoff (audit 2026-07 F4).
 *
 * AccountsSection stages a filter query via `setPendingKeysFilter`
 * before navigating; a lazily-mounted KeysSection must consume it on
 * mount, and the `cp-keys-filter` CustomEvent path must keep working
 * for an already-mounted section (and drain the staged copy so it
 * can't go stale).
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";

const keyApiList = vi.fn();
const keyOauthList = vi.fn();
const accountListBasic = vi.fn();

vi.mock("../api", () => ({
  api: {
    keyApiList: (...a: unknown[]) => keyApiList(...a),
    keyOauthList: (...a: unknown[]) => keyOauthList(...a),
    accountListBasic: (...a: unknown[]) => accountListBasic(...a),
  },
}));

vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: vi.fn() }),
}));

import { KeysSection } from "./KeysSection";
import {
  consumePendingKeysFilter,
  setPendingKeysFilter,
} from "./keys/pendingFilter";

const ALPHA_UUID = "11111111-1111-1111-1111-111111111111";
const BETA_UUID = "22222222-2222-2222-2222-222222222222";

function account(uuid: string, email: string) {
  return {
    uuid,
    email,
    org_name: null,
    subscription_type: null,
    is_cli_active: false,
    is_desktop_active: false,
    has_cli_credentials: true,
    has_desktop_profile: false,
  };
}

function apiKey(uuid: string, label: string, email: string) {
  return {
    uuid,
    label,
    token_preview: "sk-ant-api03-Abc…xyz",
    account_uuid: uuid,
    account_email: email,
    created_at: new Date().toISOString(),
    last_probed_at: null,
    last_probe_status: null,
  };
}

beforeEach(() => {
  keyApiList.mockReset().mockResolvedValue([
    apiKey(ALPHA_UUID, "alpha-key", "alpha@example.com"),
    apiKey(BETA_UUID, "beta-key", "beta@example.com"),
  ]);
  keyOauthList.mockReset().mockResolvedValue([]);
  // `matches` resolves an email filter through the accounts list
  // (uuid → email), not the row's `account_email` — provide both
  // owners so email queries actually select rows.
  accountListBasic.mockReset().mockResolvedValue([
    account(ALPHA_UUID, "alpha@example.com"),
    account(BETA_UUID, "beta@example.com"),
  ]);
  // Drain any query a previous test left staged — module singleton.
  consumePendingKeysFilter();
});

describe("KeysSection — pending filter handoff", () => {
  it("consumes a staged query on mount and pre-filters the rows", async () => {
    setPendingKeysFilter("alpha@example.com");
    render(<KeysSection />);
    await screen.findByLabelText("Copy alpha-key");
    expect(screen.queryByLabelText("Copy beta-key")).toBeNull();
    // Consumed exactly once — nothing left to go stale.
    expect(consumePendingKeysFilter()).toBeNull();
  });

  it("shows all rows when nothing is staged", async () => {
    render(<KeysSection />);
    await screen.findByLabelText("Copy alpha-key");
    expect(screen.getByLabelText("Copy beta-key")).toBeInTheDocument();
  });

  it("cp-keys-filter applies to a mounted section and drains the staged copy", async () => {
    render(<KeysSection />);
    await screen.findByLabelText("Copy alpha-key");
    // Mirror AccountsSection's dispatch: stage + fire the event.
    setPendingKeysFilter("beta@example.com");
    act(() => {
      window.dispatchEvent(
        new CustomEvent("cp-keys-filter", {
          detail: { query: "beta@example.com" },
        }),
      );
    });
    await screen.findByLabelText("Copy beta-key");
    expect(screen.queryByLabelText("Copy alpha-key")).toBeNull();
    expect(consumePendingKeysFilter()).toBeNull();
  });
});
