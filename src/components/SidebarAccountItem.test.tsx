import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { SidebarAccountItem } from "./SidebarAccountItem";
import { sampleAccount, sampleUsageEntry } from "../test/fixtures";

describe("SidebarAccountItem — usage states", () => {
  // Exercise each UsageEntry status. These tests intentionally assert
  // on user-visible text / accessible name rather than class names —
  // class names are implementation detail.
  it("renders the 5h bar when usage status is ok", () => {
    render(
      <SidebarAccountItem
        account={sampleAccount()}
        active={false}
        usageEntry={sampleUsageEntry({ status: "ok" })}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={() => {}}
      />,
    );
    // The bar label shows rounded utilization.
    expect(screen.getByText(/42%/)).toBeInTheDocument();
  });

  it("shows 'Nm ago' when usage is stale", () => {
    render(
      <SidebarAccountItem
        account={sampleAccount()}
        active={false}
        usageEntry={sampleUsageEntry({ status: "stale", age_secs: 240 })}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={() => {}}
      />,
    );
    // Bar is still there (we have data), plus a cached chip.
    expect(screen.getByText(/42%/)).toBeInTheDocument();
    expect(screen.getByText(/4m ago/)).toBeInTheDocument();
  });

  it("shows 'Token expired' placeholder with Log-in affordance", () => {
    const onLogin = vi.fn();
    render(
      <SidebarAccountItem
        account={sampleAccount()}
        active={false}
        usageEntry={sampleUsageEntry({
          status: "expired",
          usage: null,
          age_secs: null,
        })}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={onLogin}
      />,
    );
    expect(screen.getByText(/Token expired/)).toBeInTheDocument();
    const btn = screen.getByRole("button", { name: /log in again/i });
    fireEvent.click(btn);
    expect(onLogin).toHaveBeenCalledTimes(1);
  });

  it("shows 'Rate-limited' placeholder with retry countdown", () => {
    const onRefresh = vi.fn();
    render(
      <SidebarAccountItem
        account={sampleAccount()}
        active={false}
        usageEntry={sampleUsageEntry({
          status: "rate_limited",
          usage: null,
          age_secs: null,
          retry_after_secs: 45,
        })}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={() => {}}
        onRefreshUsage={onRefresh}
      />,
    );
    expect(screen.getByText(/Rate-limited/)).toBeInTheDocument();
    expect(screen.getByText(/45s/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("shows 'Couldn't fetch' placeholder with Retry action", () => {
    const onRefresh = vi.fn();
    render(
      <SidebarAccountItem
        account={sampleAccount()}
        active={false}
        usageEntry={sampleUsageEntry({
          status: "error",
          usage: null,
          age_secs: null,
          error_detail: "timeout",
        })}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={() => {}}
        onRefreshUsage={onRefresh}
      />,
    );
    expect(screen.getByText(/Couldn't fetch usage/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("renders nothing in the usage slot when usageEntry is null", () => {
    render(
      <SidebarAccountItem
        account={sampleAccount({ has_cli_credentials: false, credentials_healthy: false })}
        active={false}
        usageEntry={null}
        cliBusy={false}
        reBusy={false}
        onSelect={() => {}}
        onSwitchCli={() => {}}
        onLogin={() => {}}
      />,
    );
    // The row still renders the email + login affordance; no bar / no
    // placeholder appears.
    expect(screen.getByText("alice@example.com")).toBeInTheDocument();
    expect(screen.queryByText(/%/)).not.toBeInTheDocument();
    expect(screen.queryByText(/expired/i)).not.toBeInTheDocument();
  });
});
