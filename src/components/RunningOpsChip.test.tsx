import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RunningOpsChip, labelFor } from "./RunningOpsChip";
import type { RunningOpInfo } from "../types";
import { OP_KINDS } from "../types/ops";

function op(partial: Partial<RunningOpInfo> = {}): RunningOpInfo {
  return {
    op_id: "op-1",
    kind: "repair_resume",
    old_path: "/a/b",
    new_path: "/a/c",
    current_phase: null,
    phase_states: {},
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
            phase_states: {},
            sub_progress: [47, 168],
          }),
          op({
            op_id: "op-b",
            kind: "repair_rollback",
            current_phase: "P3",
            phase_states: {},
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
          phase_states: {},
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
          phase_states: {},
        }),
      ),
    ).toBe("Slimming abc.jsonl (P2)");
  });

  it("renders basenames from Windows-shaped paths (rules/paths.md)", () => {
    expect(
      labelFor(
        op({
          kind: "move_project",
          old_path: "C:\\Users\\me\\old-proj",
          new_path: "C:\\Users\\me\\new-proj",
        }),
      ),
    ).toBe("Renaming old-proj → new-proj");
  });
});

// The enforcement half of the 2026-08-18 fix. `OpKind::AgentRun` existed
// in Rust and was registered by `agents_run_now_start`, but the TS union
// never listed it — so `verb()`'s switch was "exhaustive" over an
// incomplete union, TypeScript raised nothing, and a live agent run
// rendered with an undefined verb while still being counted.
//
// Adding the arm fixes today. This test is what stops the next OpKind
// repeating it: it walks every kind as a runtime value, so a variant
// added to Rust and the union but not to the switch fails here.
describe("every OpKind renders a label", () => {
  it("has a non-empty, non-key label for each kind", () => {
    for (const kind of OP_KINDS) {
      const label = labelFor(op({ kind }));
      expect(label, `${kind} has no label`).toBeTruthy();
      // design.md: no internal identifiers in primary UI. The backend
      // passes the agent UUID in `old_path`, so a kind that falls
      // through to the rename label would leak it here.
      expect(label, `${kind} leaked a UUID`).not.toMatch(
        /[0-9a-f]{8}-[0-9a-f]{4}-/i,
      );
      // A missing catalog entry renders the key itself; that is a
      // failure, not a label.
      expect(label, `${kind} rendered a raw i18n key`).not.toContain("ops.");
      expect(label, `${kind} rendered undefined`).not.toContain("undefined");
    }
  });
});
