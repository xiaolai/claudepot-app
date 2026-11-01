import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Modal } from "./Modal";

describe("Modal", () => {
  // The scrim is `position: fixed` and has to cover the window. Inside a
  // query container it could be limited to that container's box.
  it("renders the scrim directly under body, whatever its parent is", () => {
    const { container } = render(
      <div className="settings-shell">
        <Modal open onClose={() => {}} aria-labelledby="t">
          <h2 id="t">Title</h2>
        </Modal>
      </div>,
    );
    const scrim = screen.getByRole("dialog").parentElement;
    expect(scrim).toHaveClass("modal-scrim");
    expect(scrim?.parentElement).toBe(document.body);
    expect(container.querySelector(".modal-scrim")).toBeNull();
  });

  // The portal leaves the app root's 13px behind, and `body` is 11px, so
  // text that names no size relies on the dialog setting one.
  it("sets the app's base font size on the dialog", () => {
    render(
      <Modal open onClose={() => {}} aria-labelledby="t">
        <h2 id="t">Title</h2>
      </Modal>,
    );
    expect(screen.getByRole("dialog").style.fontSize).toBe("var(--fs-base)");
  });
});
