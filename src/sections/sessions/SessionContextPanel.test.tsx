import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { ContextStats } from "../../types";
import { SessionContextPanel } from "./SessionContextPanel";

vi.mock("../../api", () => {
  const mock = vi.fn();
  return {
    api: { sessionContextAttribution: mock },
    __mockSessionContextAttribution: mock,
  };
});

// @ts-expect-error vi mock exposes our helper
import { __mockSessionContextAttribution } from "../../api";

function stats(overrides: Partial<ContextStats> = {}): ContextStats {
  return {
    totals: {
      claude_md: 1000,
      mentioned_file: 500,
      tool_output: 2500,
      thinking_text: 250,
      team_coordination: 0,
      user_message: 750,
    },
    injections: [
      {
        event_index: 1,
        category: "tool-output",
        label: "Read",
        tokens: 2500,
        ts: null,
        phase: 0,
      },
      {
        event_index: 0,
        category: "claude-md",
        label: "/CLAUDE.md",
        tokens: 1000,
        ts: null,
        phase: 0,
      },
    ],
    phases: [
      {
        phase_number: 0,
        start_index: 0,
        end_index: 5,
        start_ts: null,
        end_ts: null,
        summary: null,
      },
    ],
    reported_total_tokens: 42000,
    ...overrides,
  };
}

describe("SessionContextPanel", () => {
  it("renders category totals after loading", async () => {
    __mockSessionContextAttribution.mockResolvedValueOnce(stats());
    render(
      <SessionContextPanel
        filePath="/t.jsonl"
        onClose={() => {}}
        refreshSignal={0}
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("category-claude-md")).toBeInTheDocument();
    });
    expect(screen.getByTestId("category-tool-output")).toBeInTheDocument();
    // reported total appears.
    expect(screen.getByText(/42,000 total/)).toBeInTheDocument();
  });

  it("surfaces an error line on failure", async () => {
    __mockSessionContextAttribution.mockRejectedValueOnce(new Error("boom"));
    render(
      <SessionContextPanel
        filePath="/t.jsonl"
        onClose={() => {}}
        refreshSignal={0}
      />,
    );
    await waitFor(() => {
      expect(screen.getByText(/Couldn't load context/)).toBeInTheDocument();
    });
  });

  it("invokes onClose when the close button is clicked", async () => {
    __mockSessionContextAttribution.mockResolvedValueOnce(stats());
    const close = vi.fn();
    render(
      <SessionContextPanel
        filePath="/t.jsonl"
        onClose={close}
        refreshSignal={0}
      />,
    );
    const btn = await screen.findByRole("button", {
      name: "Close visible context panel",
    });
    await userEvent.click(btn);
    expect(close).toHaveBeenCalledOnce();
  });

  it("totals follow the selected phase", async () => {
    __mockSessionContextAttribution.mockResolvedValueOnce(
      stats({
        totals: {
          claude_md: 0,
          mentioned_file: 0,
          tool_output: 1000,
          thinking_text: 0,
          team_coordination: 0,
          user_message: 500,
        },
        injections: [
          {
            event_index: 0,
            category: "tool-output",
            label: "Read",
            tokens: 1000,
            ts: null,
            phase: 0,
          },
          {
            event_index: 1,
            category: "user-message",
            label: "user",
            tokens: 500,
            ts: null,
            phase: 1,
          },
        ],
        phases: [
          {
            phase_number: 0,
            start_index: 0,
            end_index: 1,
            start_ts: null,
            end_ts: null,
            summary: null,
          },
          {
            phase_number: 1,
            start_index: 2,
            end_index: 3,
            start_ts: null,
            end_ts: null,
            summary: "compacted",
          },
        ],
      }),
    );
    render(
      <SessionContextPanel
        filePath="/t.jsonl"
        onClose={() => {}}
        refreshSignal={0}
      />,
    );
    // Wait for initial render and confirm aggregate totals show up.
    const pill0 = await screen.findByRole("button", { name: "#0" });
    await userEvent.click(pill0);
    // After selecting phase 0, the phase injection tokens should match.
    // The tool-output category should show 1000 tokens, user-message 0.
    // The simplest deterministic assertion: the category row for
    // user-message should now show "0" tokens.
    const userRow = screen.getByTestId("category-user-message");
    expect(userRow.textContent ?? "").toMatch(/\b0\b/);
  });

  it("shows phase picker only when more than one phase exists", async () => {
    __mockSessionContextAttribution.mockResolvedValueOnce(
      stats({
        phases: [
          {
            phase_number: 0,
            start_index: 0,
            end_index: 2,
            start_ts: null,
            end_ts: null,
            summary: null,
          },
          {
            phase_number: 1,
            start_index: 3,
            end_index: 5,
            start_ts: null,
            end_ts: null,
            summary: "compacted",
          },
        ],
      }),
    );
    render(
      <SessionContextPanel
        filePath="/t.jsonl"
        onClose={() => {}}
        refreshSignal={0}
      />,
    );
    await waitFor(() => expect(screen.getByText("Phase")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "#0" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "#1" })).toBeInTheDocument();
  });
});
