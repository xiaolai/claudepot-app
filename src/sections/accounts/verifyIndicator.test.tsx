import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AccountSummary } from "../../types";
import { HealthFooter } from "./HealthFooter";
import { verifyLiveFor, type VerifyAllState } from "./useAccountHandlers";

function mkAccount(overrides: Partial<AccountSummary> = {}): AccountSummary {
  return {
    uuid: "acct-1",
    email: "a@example.com",
    org_name: "personal",
    subscription_type: null,
    is_cli_active: false,
    is_desktop_active: false,
    has_cli_credentials: true,
    credentials_healthy: true,
    has_desktop_profile: false,
    desktop_profile_on_disk: false,
    verify_status: "never",
    verified_email: null,
    verified_at: null,
    drift: false,
    token_status: "valid",
    token_remaining_mins: null,
    last_cli_switch: null,
    last_desktop_switch: null,
    ...overrides,
  };
}

describe("verifyLiveFor", () => {
  const idle: VerifyAllState = {
    active: false,
    done: 0,
    total: 0,
    outcomes: {},
  };

  it("returns undefined when no run is active", () => {
    expect(verifyLiveFor(idle, "acct-1")).toBeUndefined();
    // Even with a leftover outcome, an inactive run never overrides the
    // persisted status.
    expect(
      verifyLiveFor({ ...idle, outcomes: { "acct-1": "ok" } }, "acct-1"),
    ).toBeUndefined();
  });

  it("reports 'verifying' for an account that hasn't resolved this run", () => {
    const active: VerifyAllState = {
      active: true,
      done: 1,
      total: 3,
      outcomes: { "acct-2": "ok" },
    };
    expect(verifyLiveFor(active, "acct-1")).toBe("verifying");
  });

  it("reports the streamed outcome once an account resolves", () => {
    const active: VerifyAllState = {
      active: true,
      done: 2,
      total: 3,
      outcomes: { "acct-1": "drift", "acct-2": "ok" },
    };
    expect(verifyLiveFor(active, "acct-1")).toBe("drift");
  });
});

describe("HealthFooter — verification indicator", () => {
  it("shows the persisted status when no run is active", () => {
    render(<HealthFooter account={mkAccount({ verify_status: "never" })} />);
    expect(screen.getByText("not yet verified")).toBeInTheDocument();
    expect(screen.queryByText(/verifying…/i)).toBeNull();
  });

  it("shows the 'verifying…' pulse while the account is pending", () => {
    const { container } = render(
      <HealthFooter
        account={mkAccount({ verify_status: "ok", verified_at: "2026-06-01" })}
        verifyLive="verifying"
      />,
    );
    expect(screen.getByText("verifying…")).toBeInTheDocument();
    // The pulse dot is present (and carries the reduced-motion-aware
    // class), not the resolved check glyph.
    expect(container.querySelector(".cp-pulse-dot")).not.toBeNull();
    // The stale "verified" label is suppressed while checking.
    expect(screen.queryByText(/^verified/)).toBeNull();
  });

  it("flips to the streamed outcome before the persisted status updates", () => {
    // Account still says "never" on disk; the live outcome wins.
    render(
      <HealthFooter
        account={mkAccount({ verify_status: "never" })}
        verifyLive="rejected"
      />,
    );
    expect(screen.getByText("token rejected")).toBeInTheDocument();
    expect(screen.queryByText("not yet verified")).toBeNull();
    expect(screen.queryByText(/verifying…/i)).toBeNull();
  });
});
