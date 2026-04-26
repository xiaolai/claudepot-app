import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const previewSpy = vi.fn();
const executeSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    projectRemovePreview: (...args: unknown[]) => previewSpy(...args),
    projectRemoveExecute: (...args: unknown[]) => executeSpy(...args),
  },
}));

import { RemoveProjectModal } from "./RemoveProjectModal";
import type { RemoveProjectPreview } from "../../types";

function okPreview(
  overrides: Partial<RemoveProjectPreview> = {},
): RemoveProjectPreview {
  return {
    slug: "-Users-joker-myproject",
    original_path: "/Users/joker/myproject",
    bytes: 4_200_000,
    session_count: 12,
    last_modified_ms: Date.now() - 3 * 24 * 3600 * 1000,
    has_live_session: false,
    claude_json_entry_present: true,
    history_lines_count: 7,
    ...overrides,
  };
}

beforeEach(() => {
  previewSpy.mockReset();
  executeSpy.mockReset();
});

describe("RemoveProjectModal", () => {
  it("renders the three blocks with cwd verbatim and slug", async () => {
    previewSpy.mockResolvedValue(okPreview());
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={() => {}}
        onError={() => {}}
      />,
    );

    // Removing block — slug-form path under ~/.claude/projects/.
    await waitFor(() =>
      expect(
        screen.getByText("~/.claude/projects/-Users-joker-myproject/"),
      ).toBeInTheDocument(),
    );
    // Not touching block — cwd verbatim, the user's actual fear.
    expect(screen.getByText("/Users/joker/myproject")).toBeInTheDocument();
    expect(screen.getByText(/Untouched/)).toBeInTheDocument();
    // Recoverable block.
    expect(screen.getByText(/Recoverable until/)).toBeInTheDocument();
  });

  it("disables Remove until the slug is typed exactly", async () => {
    previewSpy.mockResolvedValue(okPreview());
    const user = userEvent.setup();
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={() => {}}
        onError={() => {}}
      />,
    );
    const remove = await screen.findByRole("button", { name: "Remove" });
    expect(remove).toBeDisabled();
    expect(screen.getByText(/Type -Users-joker-myproject to confirm/i)).toBeInTheDocument();

    const input = screen.getByLabelText(
      /Type project slug to confirm/i,
    );
    // Partial match — still disabled.
    await user.type(input, "-Users-joker");
    expect(remove).toBeDisabled();

    // Exact match — enabled.
    await user.type(input, "-myproject");
    expect(remove).not.toBeDisabled();
  });

  it("blocks confirm with inline reason when a live session is detected", async () => {
    previewSpy.mockResolvedValue(okPreview({ has_live_session: true }));
    const user = userEvent.setup();
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={() => {}}
        onError={() => {}}
      />,
    );
    const input = await screen.findByLabelText(
      /Type project slug to confirm/i,
    );
    // Even with the slug typed, live-session keeps Remove disabled.
    await user.type(input, "-Users-joker-myproject");
    const remove = screen.getByRole("button", { name: "Remove" });
    expect(remove).toBeDisabled();
    expect(
      screen.getByText(/Live CC session running/),
    ).toBeInTheDocument();
  });

  it("calls executeSpy on confirmed Remove and bubbles the result", async () => {
    previewSpy.mockResolvedValue(okPreview());
    executeSpy.mockResolvedValue({
      slug: "-Users-joker-myproject",
      original_path: "/Users/joker/myproject",
      bytes: 4_200_000,
      session_count: 12,
      trash_id: "20260426T120000Z-deadbeef",
      claude_json_entry_removed: true,
      history_lines_removed: 7,
    });
    const onCompleted = vi.fn();
    const user = userEvent.setup();
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={onCompleted}
        onError={() => {}}
      />,
    );
    const input = await screen.findByLabelText(
      /Type project slug to confirm/i,
    );
    await user.type(input, "-Users-joker-myproject");
    await user.click(screen.getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(executeSpy).toHaveBeenCalledWith("/Users/joker/myproject"));
    expect(onCompleted).toHaveBeenCalledWith(
      expect.objectContaining({ trash_id: "20260426T120000Z-deadbeef" }),
    );
  });

  it("calls onError when execute rejects", async () => {
    previewSpy.mockResolvedValue(okPreview());
    executeSpy.mockRejectedValue("live session");
    const onError = vi.fn();
    const user = userEvent.setup();
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={() => {}}
        onError={onError}
      />,
    );
    const input = await screen.findByLabelText(
      /Type project slug to confirm/i,
    );
    await user.type(input, "-Users-joker-myproject");
    await user.click(screen.getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith("live session"));
  });

  it("Cancel is the primary affordance", async () => {
    previewSpy.mockResolvedValue(okPreview());
    render(
      <RemoveProjectModal
        target="/Users/joker/myproject"
        onClose={() => {}}
        onCompleted={() => {}}
        onError={() => {}}
      />,
    );
    // Cancel should be visible/non-disabled before preview lands.
    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(cancel).not.toBeDisabled();
  });
});
