import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import type { SessionEvent } from "../../types";
import { SessionEventView } from "./SessionEventView";

const prefsSpy = vi.fn(() => ({ enabled: false, hideThinking: true }));
vi.mock("../../hooks/useActivityPrefs", () => ({
  useActivityPrefs: () => prefsSpy(),
}));

function thinkingEvent(text: string): SessionEvent {
  return {
    kind: "assistantThinking",
    ts: null,
    uuid: null,
    text,
  };
}

function renderView(node: ReactElement) {
  return render(node);
}

describe("SessionEventView — thinking redaction (M-11)", () => {
  it("redacts the thinking block by default and reveals on click", async () => {
    prefsSpy.mockReturnValue({ enabled: true, hideThinking: true });
    const text = "Deliberating about the edge case";
    renderView(
      <SessionEventView event={thinkingEvent(text)} searchTerm="" />,
    );
    const reveal = screen.getByRole("button", {
      name: /reveal thinking block/i,
    });
    expect(reveal).toHaveTextContent(/click to reveal/i);
    // The raw text must NOT be in the DOM before the user clicks.
    expect(screen.queryByText(text)).toBeNull();
    await userEvent.click(reveal);
    // After clicking, the body should render the raw text.
    expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("shows the full thinking block immediately when the pref is off", () => {
    prefsSpy.mockReturnValue({ enabled: true, hideThinking: false });
    const text = "Open thinking content";
    renderView(
      <SessionEventView event={thinkingEvent(text)} searchTerm="" />,
    );
    // No reveal button — the body renders straight away.
    expect(
      screen.queryByRole("button", { name: /reveal thinking block/i }),
    ).toBeNull();
    expect(screen.getByText(text)).toBeInTheDocument();
  });
});
