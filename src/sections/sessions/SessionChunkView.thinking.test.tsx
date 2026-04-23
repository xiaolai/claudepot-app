import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ThinkingDetails } from "./SessionChunkView";

/**
 * Guards the C-1-A follow-up fix: chunks-mode (the DEFAULT transcript
 * view) must honor `activity_hide_thinking`. The earlier M-11 wire
 * only applied in raw mode, which nobody uses by default — so the
 * pref was observably dead until the view-mode switch.
 */

const prefsSpy = vi.fn(() => ({ enabled: true, hideThinking: true }));
vi.mock("../../hooks/useActivityPrefs", () => ({
  useActivityPrefs: () => prefsSpy(),
}));

describe("ThinkingDetails (chunks mode)", () => {
  it("redacts the block when hideThinking is on and reveals on click", async () => {
    prefsSpy.mockReturnValue({ enabled: true, hideThinking: true });
    const text = "Deep thoughts in chunks mode";
    render(<ThinkingDetails text={text} searchTerm="" />);
    const reveal = screen.getByRole("button", {
      name: /reveal thinking block/i,
    });
    expect(reveal).toHaveTextContent(/click to reveal/i);
    expect(screen.queryByText(text)).toBeNull();
    await userEvent.click(reveal);
    expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("shows the details element straight away when hideThinking is off", () => {
    prefsSpy.mockReturnValue({ enabled: true, hideThinking: false });
    const text = "No redaction please";
    render(<ThinkingDetails text={text} searchTerm="" />);
    expect(
      screen.queryByRole("button", { name: /reveal thinking block/i }),
    ).toBeNull();
    expect(screen.getByText(text)).toBeInTheDocument();
    expect(document.querySelector("details[open]")).toBeInTheDocument();
  });
});
