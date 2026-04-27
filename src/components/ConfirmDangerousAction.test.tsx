import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConfirmDangerousAction } from "./ConfirmDangerousAction";

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

