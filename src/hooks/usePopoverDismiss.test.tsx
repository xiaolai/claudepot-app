import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { usePopoverDismiss } from "./usePopoverDismiss";

/** Minimal popover host: a root div with a toggle and a popover
 *  panel, plus an outside sibling to click. */
function Host({ onDismiss }: { onDismiss?: () => void }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  usePopoverDismiss(rootRef, open, () => {
    onDismiss?.();
    setOpen(false);
  });
  return (
    <div>
      <div ref={rootRef} data-testid="root">
        <button type="button" onClick={() => setOpen((o) => !o)}>
          toggle
        </button>
        {open && <div data-testid="popover">popover</div>}
      </div>
      <button type="button" data-testid="outside">
        outside
      </button>
    </div>
  );
}

/** The mousedown listener is wired behind a 0ms timeout — flush it. */
async function flushWiring() {
  await new Promise((r) => setTimeout(r, 0));
}

describe("usePopoverDismiss", () => {
  it("dismisses on mousedown outside the root", async () => {
    render(<Host />);
    fireEvent.click(screen.getByText("toggle"));
    expect(screen.getByTestId("popover")).toBeInTheDocument();
    await flushWiring();
    fireEvent.mouseDown(screen.getByTestId("outside"));
    expect(screen.queryByTestId("popover")).toBeNull();
  });

  it("does not dismiss on mousedown inside the root", async () => {
    render(<Host />);
    fireEvent.click(screen.getByText("toggle"));
    await flushWiring();
    fireEvent.mouseDown(screen.getByTestId("popover"));
    expect(screen.getByTestId("popover")).toBeInTheDocument();
  });

  it("dismisses on Escape", async () => {
    render(<Host />);
    fireEvent.click(screen.getByText("toggle"));
    await flushWiring();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("popover")).toBeNull();
  });

  it("does nothing while closed", async () => {
    const onDismiss = vi.fn();
    render(<Host onDismiss={onDismiss} />);
    await flushWiring();
    fireEvent.mouseDown(screen.getByTestId("outside"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("tears listeners down after unmount", async () => {
    const onDismiss = vi.fn();
    const { unmount } = render(<Host onDismiss={onDismiss} />);
    fireEvent.click(screen.getByText("toggle"));
    await flushWiring();
    unmount();
    fireEvent.mouseDown(document.body);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
