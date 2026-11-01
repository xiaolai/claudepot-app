/**
 * Settings → Locks.
 *
 * Breaking a lock deletes it without checking whether its holder is
 * alive, so the pane confirms first — as Projects → Maintenance and
 * `project repair --break-lock` (which asks for `-y`) already did.
 * The pane once fired on the first click. It also asked for a "lock
 * file path" while the backend resolves a project path or slug, so a
 * real lock-file path always failed with "no lock file found".
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const breakLockSpy = vi.fn();

vi.mock("../api", () => ({
  api: {
    repairBreakLock: (...a: unknown[]) => breakLockSpy(...a),
  },
}));

import { LocksPane } from "./SettingsSection";

const PROJECT = "/Users/me/old-project";

describe("LocksPane", () => {
  beforeEach(() => {
    breakLockSpy.mockReset();
    breakLockSpy.mockResolvedValue({
      prior_pid: 42,
      prior_hostname: "box",
      audit_path: "/tmp/broken-lock.json",
    });
  });

  it("asks for the project path, not a lock-file path", () => {
    render(<LocksPane pushToast={vi.fn()} />);
    expect(screen.getByText("Project path")).toBeInTheDocument();
    expect(screen.queryByText("Lock file path")).toBeNull();
  });

  it("confirms before breaking, and breaks only on confirm", async () => {
    const pushToast = vi.fn();
    render(<LocksPane pushToast={pushToast} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: PROJECT },
    });
    fireEvent.click(screen.getByRole("button", { name: "Break lock" }));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(PROJECT);
    expect(breakLockSpy).not.toHaveBeenCalled();

    const confirm = screen
      .getAllByRole("button", { name: "Break lock" })
      .find((b) => dialog.contains(b));
    fireEvent.click(confirm!);

    await waitFor(() => expect(breakLockSpy).toHaveBeenCalledWith(PROJECT));
    expect(pushToast).toHaveBeenCalledWith("info", expect.any(String));
  });

  it("cancel closes the dialog without breaking", async () => {
    render(<LocksPane pushToast={vi.fn()} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: PROJECT },
    });
    fireEvent.click(screen.getByRole("button", { name: "Break lock" }));
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(breakLockSpy).not.toHaveBeenCalled();
  });
});
