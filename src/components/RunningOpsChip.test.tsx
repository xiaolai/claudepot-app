import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RunningOpsChip, labelFor } from "./RunningOpsChip";
import type { RunningOpInfo } from "../types";

function op(partial: Partial<RunningOpInfo> = {}): RunningOpInfo {
  return {
    op_id: "op-1",
    kind: "repair_resume",
    old_path: "/a/b",
    new_path: "/a/c",
    current_phase: null,
    sub_progress: null,
    status: "running",
    started_unix_secs: 0,
    last_error: null,
    move_result: null,
    clean_result: null,
    failed_journal_id: null,
    ...partial,
  };
}

describe("RunningOpsChip", () => {
  it("renders nothing when no running ops", () => {
    const { container } = render(
      <RunningOpsChip ops={[]} onReopen={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders nothing when all ops have non-running status", () => {
    const { container } = render(
      <RunningOpsChip
        ops={[op({ status: "complete" })]}
        onReopen={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("uses singular label for one op", () => {
    render(<RunningOpsChip ops={[op()]} onReopen={() => {}} />);
    expect(screen.getByText("1 op")).toBeInTheDocument();
  });

  it("uses plural label for multiple ops", () => {
    render(
      <RunningOpsChip
        ops={[
          op({ op_id: "a" }),
          op({ op_id: "b", kind: "repair_rollback" }),
        ]}
        onReopen={() => {}}
      />,
    );
    expect(screen.getByText("2 ops")).toBeInTheDocument();
  });

  it("opens the popover on chip click and lists each running op", async () => {
    const user = userEvent.setup();
    render(
      <RunningOpsChip
        ops={[
          op({
            op_id: "op-a",
            current_phase: "P6",
            sub_progress: [47, 168],
          }),
          op({
            op_id: "op-b",
            kind: "repair_rollback",
            current_phase: "P3",
          }),
        ]}
        onReopen={() => {}}
      />,
    );
    await user.click(screen.getByRole("button", { name: /background operation/i }));
    expect(
      screen.getByText(/Resuming.*P6: 47\/168 files/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Rolling back.*P3/)).toBeInTheDocument();
  });

  it("clicking a popover row fires onReopen with the op_id and closes", async () => {
    const user = userEvent.setup();
    const reopen = vi.fn();
    render(
      <RunningOpsChip
        ops={[op({ op_id: "op-xyz", current_phase: "P1" })]}
        onReopen={reopen}
      />,
    );
    await user.click(screen.getByRole("button", { name: /background operation/i }));
    await user.click(screen.getByRole("menuitem"));
    expect(reopen).toHaveBeenCalledWith("op-xyz");
    // Popover closes — the menuitem unmounts.
    expect(screen.queryByRole("menuitem")).toBeNull();
  });

  it("Escape closes the popover", async () => {
    const user = userEvent.setup();
    render(<RunningOpsChip ops={[op()]} onReopen={() => {}} />);
    await user.click(screen.getByRole("button", { name: /background operation/i }));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("labelFor", () => {
  it("formats verify_all without paths", () => {
    expect(
      labelFor(op({ kind: "verify_all", old_path: "", new_path: "" })),
    ).toBe("Verifying  → ");
  });

  it("formats clean_projects with sub-progress", () => {
    expect(
      labelFor(
        op({
          kind: "clean_projects",
          current_phase: "scan",
          sub_progress: [3, 10],
        }),
      ),
    ).toBe("Cleaning projects (3/10)");
  });

  it("formats session_prune with sub-progress", () => {
    expect(
      labelFor(
        op({ kind: "session_prune", sub_progress: [5, 20] }),
      ),
    ).toBe("Pruning sessions (5/20)");
  });

  it("formats session_slim with file basename", () => {
    expect(
      labelFor(
        op({
          kind: "session_slim",
          old_path: "/p/abc.jsonl",
          current_phase: "P2",
        }),
      ),
    ).toBe("Slimming abc.jsonl (P2)");
  });
});
