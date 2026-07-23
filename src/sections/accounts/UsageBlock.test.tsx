import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { UsageEntry } from "../../types";
import { UsageBlock } from "./UsageBlock";

function mkEntry(overrides: Partial<UsageEntry> = {}): UsageEntry {
  return {
    status: "expired",
    usage: null,
    age_secs: null,
    retry_after_secs: null,
    error_detail: null,
    ...overrides,
  };
}

describe("UsageBlock contextual actions", () => {
  it("renders Verify + Refresh on an expired card and wires each click", () => {
    const onVerify = vi.fn();
    const onRefresh = vi.fn();
    render(
      <UsageBlock entry={mkEntry()} onVerify={onVerify} onRefresh={onRefresh} />,
    );

    expect(screen.getByText(/token expired/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Verify" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onVerify).toHaveBeenCalledTimes(1);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("suppresses the message and actions when the anomaly banner is shown", () => {
    render(
      <UsageBlock
        entry={mkEntry()}
        anomalyShown
        onVerify={vi.fn()}
        onRefresh={vi.fn()}
      />,
    );

    // The whole block renders null so the card shows a single signal.
    expect(screen.queryByText(/token expired/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("renders a single Retry on a fetch-error card and shows the detail", () => {
    const onRefresh = vi.fn();
    render(
      <UsageBlock
        entry={mkEntry({ status: "error", error_detail: "500 upstream" })}
        onVerify={vi.fn()}
        onRefresh={onRefresh}
      />,
    );

    expect(screen.getByText(/500 upstream/)).toBeInTheDocument();
    // Error state offers Retry only — Verify wouldn't heal a fetch error.
    expect(
      screen.queryByRole("button", { name: "Verify" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("flips to a disabled busy label while an async action runs, then back", async () => {
    let resolve!: () => void;
    const onVerify = vi.fn(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    render(
      <UsageBlock entry={mkEntry()} onVerify={onVerify} onRefresh={vi.fn()} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Verify" }));
    expect(onVerify).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Verifying…" })).toBeDisabled();

    resolve();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Verify" })).toBeEnabled(),
    );
  });

  it("falls back to a plain message when no action handlers are provided", () => {
    render(<UsageBlock entry={mkEntry()} />);
    expect(screen.getByText(/token expired/i)).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
