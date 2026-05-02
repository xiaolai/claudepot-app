import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const memoryHealthGetMock = vi.fn();

vi.mock("../../api", () => ({
  api: {
    memoryHealthGet: (...args: unknown[]) => memoryHealthGetMock(...args),
  },
}));

import { MemoryHealthPanel } from "./MemoryHealthPanel";
import type { MemoryHealthReport } from "../../types";

function blownReport(): MemoryHealthReport {
  // CLAUDE.md is 250 lines = 50 past cutoff. MEMORY.md is missing.
  return {
    claude_md: {
      path: "/home/user/.claude/CLAUDE.md",
      missing: false,
      line_count: 250,
      char_count: 12_000,
      lines_past_cutoff: 50,
      chars_past_cutoff: 2_400,
      est_tokens: 3_000,
    },
    memory_md: {
      path: "/home/user/.claude/memory/MEMORY.md",
      missing: true,
      line_count: 0,
      char_count: 0,
      lines_past_cutoff: 0,
      chars_past_cutoff: 0,
      est_tokens: 0,
    },
    line_cutoff: 200,
  };
}

function cleanReport(): MemoryHealthReport {
  return {
    claude_md: {
      path: "/x/CLAUDE.md",
      missing: false,
      line_count: 100,
      char_count: 4_000,
      lines_past_cutoff: 0,
      chars_past_cutoff: 0,
      est_tokens: 1_000,
    },
    memory_md: {
      path: "/x/memory/MEMORY.md",
      missing: false,
      line_count: 5,
      char_count: 200,
      lines_past_cutoff: 0,
      chars_past_cutoff: 0,
      est_tokens: 50,
    },
    line_cutoff: 200,
  };
}

describe("MemoryHealthPanel", () => {
  beforeEach(() => {
    memoryHealthGetMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders metrics for both files when content is healthy", async () => {
    memoryHealthGetMock.mockResolvedValue(cleanReport());
    render(<MemoryHealthPanel />);
    await waitFor(() =>
      expect(screen.getByText("CLAUDE.md")).toBeInTheDocument(),
    );
    expect(screen.getByText("MEMORY.md")).toBeInTheDocument();
    // Lines tile shows the formatted count.
    expect(screen.getByText("100")).toBeInTheDocument();
    // Both files have zero past-cutoff → "all visible" caption appears.
    const allVisible = screen.getAllByText(/all visible/i);
    expect(allVisible.length).toBe(2);
  });

  it("flags lines past the cutoff with a warning value", async () => {
    memoryHealthGetMock.mockResolvedValue(blownReport());
    render(<MemoryHealthPanel />);
    await waitFor(() =>
      expect(screen.getByText("50")).toBeInTheDocument(),
    );
    // The "Past line 200" tile renders the cutoff in its label.
    expect(screen.getByText(/Past line 200/i)).toBeInTheDocument();
    // Invisible-bytes caption reports the past-cutoff size in KB.
    expect(screen.getByText(/2\.3 KB invisible/i)).toBeInTheDocument();
  });

  it("renders the missing-file state when a file is absent", async () => {
    memoryHealthGetMock.mockResolvedValue(blownReport());
    render(<MemoryHealthPanel />);
    await waitFor(() =>
      expect(screen.getByText(/File not present/i)).toBeInTheDocument(),
    );
  });

  it("re-fetches when the user clicks Refresh", async () => {
    memoryHealthGetMock.mockResolvedValue(cleanReport());
    render(<MemoryHealthPanel />);
    await waitFor(() =>
      expect(memoryHealthGetMock).toHaveBeenCalledTimes(1),
    );
    const btn = screen.getByRole("button", { name: /Refresh/i });
    const user = userEvent.setup();
    await user.click(btn);
    await waitFor(() =>
      expect(memoryHealthGetMock).toHaveBeenCalledTimes(2),
    );
  });

  it("surfaces backend errors as an alert role", async () => {
    memoryHealthGetMock.mockRejectedValue(new Error("permission denied"));
    render(<MemoryHealthPanel />);
    await waitFor(() => {
      const alert = screen.getByRole("alert");
      expect(alert).toHaveTextContent(/permission denied/i);
    });
  });
});
