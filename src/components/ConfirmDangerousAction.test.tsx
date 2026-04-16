import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConfirmDangerousAction } from "./ConfirmDangerousAction";
import { RunningOpStrip } from "./RunningOpStrip";
import type { RunningOpInfo } from "../types";

describe("ConfirmDangerousAction", () => {
  it("renders consequences + Cancel/Confirm", () => {
    render(
      <ConfirmDangerousAction
        title="Break lock?"
        consequences={<p>This will delete the lock file.</p>}
        confirmLabel="Break"
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    );
    expect(screen.getByRole("heading", { name: "Break lock?" })).toBeInTheDocument();
    expect(screen.getByText(/delete the lock file/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Break" })).toBeInTheDocument();
  });

  it("Escape key triggers onCancel", () => {
    const cancel = vi.fn();
    render(
      <ConfirmDangerousAction
        title="T"
        consequences={<p>x</p>}
        confirmLabel="Go"
        onCancel={cancel}
        onConfirm={() => {}}
      />,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(cancel).toHaveBeenCalled();
  });

  it("type-to-confirm keeps the Confirm button disabled until exact match", async () => {
    const user = userEvent.setup();
    const confirm = vi.fn();
    render(
      <ConfirmDangerousAction
        title="Abandon?"
        consequences={<p>ow</p>}
        confirmLabel="Abandon"
        typeToConfirm="ABANDON"
        onCancel={() => {}}
        onConfirm={confirm}
      />,
    );
    const btn = screen.getByRole("button", { name: "Abandon" });
    expect(btn).toBeDisabled();

    const input = screen.getByLabelText(/Type/);
    await user.type(input, "abandon"); // wrong case
    expect(btn).toBeDisabled();

    await user.clear(input);
    await user.type(input, "ABANDON");
    expect(btn).toBeEnabled();

    await user.click(btn);
    expect(confirm).toHaveBeenCalled();
  });

  it("dialog has role=dialog and aria-modal", () => {
    render(
      <ConfirmDangerousAction
        title="T"
        consequences={<p>x</p>}
        confirmLabel="Go"
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    );
    const dlg = screen.getByRole("dialog");
    expect(dlg).toHaveAttribute("aria-modal", "true");
  });
});

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
    ...partial,
  };
}

describe("RunningOpStrip", () => {
  it("renders nothing when no running ops", () => {
    const { container } = render(
      <RunningOpStrip ops={[]} onReopen={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders nothing when all ops are complete", () => {
    const { container } = render(
      <RunningOpStrip
        ops={[op({ status: "complete" })]}
        onReopen={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders one row per running op with phase + sub-progress label", () => {
    render(
      <RunningOpStrip
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
    expect(screen.getByText(/Resuming.*P6: 47\/168 files/)).toBeInTheDocument();
    expect(screen.getByText(/Rolling back.*P3/)).toBeInTheDocument();
  });

  it("clicking a row fires onReopen with the op_id", async () => {
    const user = userEvent.setup();
    const reopen = vi.fn();
    render(
      <RunningOpStrip
        ops={[op({ op_id: "op-xyz" })]}
        onReopen={reopen}
      />,
    );
    await user.click(screen.getByRole("button"));
    expect(reopen).toHaveBeenCalledWith("op-xyz");
  });
});
