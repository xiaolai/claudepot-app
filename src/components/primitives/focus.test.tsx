/**
 * Paper-mono primitives render their focusable `<button>` with a
 * stable `pm-focus` class so `App.css`'s
 * `.pm-focus:focus-visible { box-shadow: var(--focus-ring) }` rule
 * can apply the design-system keyboard-focus ring.
 *
 * The inline-style approach used by these primitives can't express
 * `:focus-visible` itself — no pseudo-classes in a React `style={}`
 * object — so the class is the bridge. Losing it silently would
 * remove the keyboard-focus ring across the entire app; this test
 * guards against that regression.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { SidebarItem } from "./SidebarItem";
import { NF } from "../../icons";

describe("paper-mono focus ring", () => {
  it("Button carries pm-focus on the underlying element", () => {
    const { container } = render(<Button>Label</Button>);
    const btn = container.querySelector("button");
    expect(btn).not.toBeNull();
    expect(btn!.className).toContain("pm-focus");
  });

  it("IconButton carries pm-focus on the underlying element", () => {
    const { container } = render(<IconButton glyph={NF.refresh} />);
    const btn = container.querySelector("button");
    expect(btn).not.toBeNull();
    expect(btn!.className).toContain("pm-focus");
  });

  it("SidebarItem carries pm-focus on the underlying element", () => {
    const { container } = render(<SidebarItem label="Test" />);
    const btn = container.querySelector("button");
    expect(btn).not.toBeNull();
    expect(btn!.className).toContain("pm-focus");
  });

  it("Button consumer style prop does not clobber the focus class", () => {
    // The primitive must not let callers drop pm-focus by passing
    // className (they can't today — prop not accepted — but guard
    // anyway via explicit class assertion).
    const { container } = render(
      <Button style={{ width: 100 }}>Styled</Button>,
    );
    const btn = container.querySelector("button");
    expect(btn!.className.split(/\s+/)).toContain("pm-focus");
  });
});
