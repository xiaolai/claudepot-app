import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { NF } from "../../icons";

/**
 * A disabled button must not paint its hover state.
 *
 * Both primitives tracked hover in `useState` and set it on
 * `onMouseEnter` unconditionally, while the paint function never saw
 * `disabled`. So a disabled button filled with `--bg-hover` under the
 * pointer — the exact affordance that says "clickable" — while the
 * click handler was correctly suppressed. The user gets hover
 * feedback and no response, twice, and concludes the app is broken.
 * It affected every disabled button in the product.
 *
 * These assert the *computed background*, not the presence of a
 * guard, so a future refactor that moves the paint into CSS keeps
 * them meaningful.
 */
describe("disabled buttons do not paint hover", () => {
  it("Button: background is unchanged after mouseEnter", () => {
    render(
      <Button variant="ghost" disabled onClick={() => {}}>
        Disabled
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Disabled" });
    const before = getComputedStyle(btn).background;
    fireEvent.mouseEnter(btn);
    expect(getComputedStyle(btn).background).toBe(before);
  });

  it("Button: an ENABLED one still responds, so the guard is not a blanket kill", () => {
    render(
      <Button variant="ghost" onClick={() => {}}>
        Live
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Live" });
    const before = getComputedStyle(btn).background;
    fireEvent.mouseEnter(btn);
    expect(getComputedStyle(btn).background).not.toBe(before);
  });

  it("Button: mouseDown on a disabled button does not shift it", () => {
    render(
      <Button disabled onClick={() => {}}>
        Disabled
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Disabled" });
    const before = getComputedStyle(btn).transform;
    fireEvent.mouseDown(btn);
    expect(getComputedStyle(btn).transform).toBe(before);
  });

  it("IconButton: background is unchanged after mouseEnter", () => {
    render(
      <IconButton
        glyph={NF.check}
        title="Confirm"
        aria-label="Confirm"
        disabled
        onClick={() => {}}
      />,
    );
    const btn = screen.getByRole("button", { name: "Confirm" });
    const before = getComputedStyle(btn).background;
    fireEvent.mouseEnter(btn);
    expect(getComputedStyle(btn).background).toBe(before);
  });

  it("IconButton: an ENABLED one still responds", () => {
    render(
      <IconButton
        glyph={NF.check}
        title="Confirm"
        aria-label="Confirm"
        onClick={() => {}}
      />,
    );
    const btn = screen.getByRole("button", { name: "Confirm" });
    const before = getComputedStyle(btn).background;
    fireEvent.mouseEnter(btn);
    expect(getComputedStyle(btn).background).not.toBe(before);
  });

  /**
   * The case the setter-only fix misses. A button that disables while
   * the pointer is already on it would keep a stale `hover === true`
   * and paint it, because `onMouseEnter` never fires again to be
   * guarded. This is the common shape, not an exotic one: buttons
   * disable in response to the very click happening under the cursor.
   */
  it("Button: hover acquired before disabling is not painted after", () => {
    const { rerender } = render(
      <Button variant="ghost" onClick={() => {}}>
        Save
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Save" });
    const rest = getComputedStyle(btn).background;

    fireEvent.mouseEnter(btn);
    expect(getComputedStyle(btn).background).not.toBe(rest);

    rerender(
      <Button variant="ghost" disabled onClick={() => {}}>
        Save
      </Button>,
    );
    expect(getComputedStyle(btn).background).toBe(rest);
  });
});
