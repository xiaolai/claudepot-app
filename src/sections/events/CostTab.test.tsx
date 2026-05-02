import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const localUsageAggregateMock = vi.fn();
const pricingRefreshMock = vi.fn();
const pricingTierSetMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    localUsageAggregate: (...args: unknown[]) =>
      localUsageAggregateMock(...args),
    pricingRefresh: (...args: unknown[]) => pricingRefreshMock(...args),
    pricingTierSet: (...args: unknown[]) => pricingTierSetMock(...args),
  },
}));

import {
  CostTab,
  cacheHitRate,
  formatHitRate,
  shortModelId,
} from "./CostTab";
import type { LocalUsageReport } from "../../types";

function emptyReport(): LocalUsageReport {
  return {
    window: { from_ms: null, to_ms: null },
    rows: [],
    totals: {
      session_count: 0,
      first_active_ms: null,
      last_active_ms: null,
      tokens_input: 0,
      tokens_output: 0,
      tokens_cache_creation: 0,
      tokens_cache_read: 0,
      cost_usd: null,
      unpriced_sessions: 0,
      models_by_session: {},
    },
    pricing_source: "bundled · verified 2026-01-15",
    pricing_error: null,
    pricing_tier: "anthropic_api",
  };
}

function reportWithRows(): LocalUsageReport {
  return {
    window: { from_ms: 1, to_ms: 2 },
    rows: [
      {
        project_path: "/work/foo",
        session_count: 12,
        first_active_ms: 1_000,
        last_active_ms: 5_000,
        tokens_input: 1_500_000,
        tokens_output: 750_000,
        tokens_cache_creation: 100_000,
        tokens_cache_read: 8_000_000,
        cost_usd: 42.5,
        unpriced_sessions: 0,
        models_by_session: {
          "claude-opus-4-7": 10,
          "claude-sonnet-4-6": 3,
        },
      },
      {
        project_path: "/work/bar",
        session_count: 3,
        first_active_ms: 1_000,
        last_active_ms: 4_000,
        tokens_input: 50_000,
        tokens_output: 10_000,
        tokens_cache_creation: 0,
        tokens_cache_read: 200_000,
        cost_usd: null,
        unpriced_sessions: 3,
        models_by_session: {},
      },
    ],
    totals: {
      session_count: 15,
      first_active_ms: 1_000,
      last_active_ms: 5_000,
      tokens_input: 1_550_000,
      tokens_output: 760_000,
      tokens_cache_creation: 100_000,
      tokens_cache_read: 8_200_000,
      cost_usd: 42.5,
      unpriced_sessions: 3,
      models_by_session: {
        "claude-opus-4-7": 10,
        "claude-sonnet-4-6": 3,
      },
    },
    pricing_source: "bundled · verified 2026-01-15",
    pricing_error: null,
    pricing_tier: "anthropic_api",
  };
}

describe("CostTab", () => {
  beforeEach(() => {
    localUsageAggregateMock.mockReset();
    pricingRefreshMock.mockReset();
    pricingTierSetMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the empty state when no sessions in the window", async () => {
    localUsageAggregateMock.mockResolvedValue(emptyReport());
    render(<CostTab />);
    await waitFor(() =>
      expect(screen.getByText(/No sessions in this window/i)).toBeInTheDocument(),
    );
    // Empty totals: cost dash, sessions zero. The "0" is the literal
    // session count rendered in the Sessions tile.
    expect(screen.getByText("Sessions")).toBeInTheDocument();
    expect(screen.getByText("0")).toBeInTheDocument();
  });

  it("renders summary tiles + per-project rows with formatted values", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    render(<CostTab />);
    // Tiles + row both render `$42.50` so use getAllByText.
    await waitFor(() =>
      expect(screen.getAllByText("$42.50").length).toBeGreaterThanOrEqual(2),
    );
    expect(screen.getByText("Total cost")).toBeInTheDocument();
    expect(screen.getByText("install-wide")).toBeInTheDocument();
    // Project rows — `displayPath` renders the basename; the full
    // `project_path` lives on the cell's `title` attribute for hover
    // disclosure. Asserting both: visible basename + title carries
    // the full path that callers can copy.
    const fooCell = screen.getByText("foo");
    expect(fooCell).toBeInTheDocument();
    expect(fooCell.getAttribute("title")).toBe("/work/foo");
    const barCell = screen.getByText("bar");
    expect(barCell).toBeInTheDocument();
    expect(barCell.getAttribute("title")).toBe("/work/bar");
    expect(screen.getByText("n/a")).toBeInTheDocument();
    // Format check: 1.5M renders as "1.50M"
    expect(screen.getAllByText(/^1\.5(0)?M$/).length).toBeGreaterThan(0);
  });

  it("shows the unpriced footer + Refresh prices button when count > 0", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    pricingRefreshMock.mockResolvedValue({});
    render(<CostTab />);
    await waitFor(() =>
      expect(screen.getByText(/3 of 15 sessions used a model/i)).toBeInTheDocument(),
    );
    const btn = screen.getByRole("button", { name: /Refresh prices/i });
    const user = userEvent.setup();
    // Reset call count BEFORE the click so the assertion isn't fooled
    // by the initial mount fetch.
    localUsageAggregateMock.mockClear();
    await user.click(btn);
    // Click triggers pricingRefresh, then the report re-fetches.
    await waitFor(() =>
      expect(pricingRefreshMock).toHaveBeenCalledTimes(1),
    );
    expect(localUsageAggregateMock).toHaveBeenCalled();
  });

  it("renders cache hit % column and model badges per row", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    render(<CostTab />);
    // Hit rate for /work/foo:
    //   cache_read 8M / (input 1.5M + cache_create 0.1M + cache_read 8M)
    //   = 8 / 9.6 ≈ 83%
    await waitFor(() =>
      expect(screen.getAllByText("83%").length).toBeGreaterThan(0),
    );
    // /work/bar: cache_read 200k / (50k + 0 + 200k) = 200/250 = 80%
    expect(screen.getByText("80%")).toBeInTheDocument();

    // Model badges render with stripped `claude-` prefix and `·count` suffix.
    expect(screen.getByText("opus-4-7")).toBeInTheDocument();
    expect(screen.getByText("sonnet-4-6")).toBeInTheDocument();
    expect(screen.getByText("·10")).toBeInTheDocument();
    expect(screen.getByText("·3")).toBeInTheDocument();
  });

  it("totals tile shows install-wide cache hit rate", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    render(<CostTab />);
    // Totals: cache_read 8.2M / (1.55M + 0.1M + 8.2M) = 8.2 / 9.85 ≈ 83%
    await waitFor(() =>
      expect(screen.getByText(/cache hit 83%/i)).toBeInTheDocument(),
    );
  });

  it("cacheHitRate handles zero-input rows by returning null", () => {
    expect(
      cacheHitRate({
        tokens_input: 0,
        tokens_cache_creation: 0,
        tokens_cache_read: 0,
      }),
    ).toBeNull();
    expect(formatHitRate(null)).toBe("—");
    // Pure cache-read row → 100%.
    expect(
      cacheHitRate({
        tokens_input: 0,
        tokens_cache_creation: 0,
        tokens_cache_read: 1_000,
      }),
    ).toBe(1);
    // No cache at all → 0%.
    expect(
      cacheHitRate({
        tokens_input: 1_000,
        tokens_cache_creation: 0,
        tokens_cache_read: 0,
      }),
    ).toBe(0);
  });

  it("shortModelId strips the claude- prefix when present", () => {
    expect(shortModelId("claude-opus-4-7")).toBe("opus-4-7");
    expect(shortModelId("claude-sonnet-4-6")).toBe("sonnet-4-6");
    // Unknown shape passes through unchanged.
    expect(shortModelId("foo-bar")).toBe("foo-bar");
  });

  it("renders the active pricing tier in the source pill and the picker", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    render(<CostTab />);
    await waitFor(() =>
      expect(localUsageAggregateMock).toHaveBeenCalled(),
    );
    // Tier label appears in the source pill alongside the source text.
    expect(
      screen.getByText(/Anthropic API · bundled · verified 2026-01-15/i),
    ).toBeInTheDocument();
    // Picker is hydrated to the report's tier.
    const select = screen.getByLabelText("Tier") as HTMLSelectElement;
    expect(select.value).toBe("anthropic_api");
  });

  it("changing the tier picker calls pricingTierSet and re-fetches", async () => {
    localUsageAggregateMock.mockResolvedValue(reportWithRows());
    pricingTierSetMock.mockResolvedValue(undefined);
    render(<CostTab />);
    await waitFor(() =>
      expect(localUsageAggregateMock).toHaveBeenCalled(),
    );
    const select = screen.getByLabelText("Tier") as HTMLSelectElement;
    const user = userEvent.setup();
    localUsageAggregateMock.mockClear();
    await user.selectOptions(select, "aws_bedrock");
    await waitFor(() =>
      expect(pricingTierSetMock).toHaveBeenCalledWith("aws_bedrock"),
    );
    // The setter triggers a re-fetch so the new tier label lands.
    expect(localUsageAggregateMock).toHaveBeenCalled();
  });

  it("changing the window selector triggers a re-fetch with the new spec", async () => {
    localUsageAggregateMock.mockResolvedValue(emptyReport());
    render(<CostTab />);
    // Initial fetch on mount uses 7d default.
    await waitFor(() =>
      expect(localUsageAggregateMock).toHaveBeenCalledWith({
        kind: "lastDays",
        days: 7,
      }),
    );

    const user = userEvent.setup();
    const select = screen.getByLabelText("Window") as HTMLSelectElement;
    localUsageAggregateMock.mockClear();
    await user.selectOptions(select, "all");
    await waitFor(() =>
      expect(localUsageAggregateMock).toHaveBeenCalledWith({ kind: "all" }),
    );

    localUsageAggregateMock.mockClear();
    await user.selectOptions(select, "30d");
    await waitFor(() =>
      expect(localUsageAggregateMock).toHaveBeenCalledWith({
        kind: "lastDays",
        days: 30,
      }),
    );
  });
});
