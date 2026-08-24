/**
 * Two focus mechanisms, and each primitive uses the one that matches
 * what it is.
 *
 * `button`-shaped primitives (`Button`, `IconButton`, `SidebarItem`, …)
 * render their focusable `<button>` with a stable `pm-focus` class so
 * `tokens.css`'s `.pm-focus:focus-visible { box-shadow: var(--focus-ring) }`
 * rule can apply. The inline-style approach these primitives use can't
 * express `:focus-visible` itself — no pseudo-classes in a React
 * `style={}` object — so the class is the bridge. Losing it silently
 * would remove the keyboard-focus ring across the entire app; that half
 * of this file guards against the regression.
 *
 * `Input` and `Textarea` do NOT carry `pm-focus`. `tokens.css` documents
 * two separate treatments — a box-shadow ring for "filled chrome
 * controls" and an outline for "input/list/row controls" — and `Input`
 * ignored the split for exactly as long as this test asserted it should:
 * `pm-focus`'s 3px ring, stacked on the wrapper's own accent-coloured
 * border, drew a doubled, bleeding indicator with no vertical padding to
 * contain it. They track focus in React state and draw a single outline
 * via `fieldChrome.ts` instead — the other half of this file guards
 * THAT contract: that the wrapper visibly changes on focus, and that it
 * does so through `outline`, not the button ring.
 */
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { Input } from "./Input";
import { Textarea } from "./Textarea";
import { SidebarItem } from "./SidebarItem";
import { NF } from "../../icons";

describe("paper-mono focus ring — button-shaped primitives use pm-focus", () => {
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

describe("paper-mono focus ring — Input / Textarea use an outline, not the button ring", () => {
  it("neither carries pm-focus", () => {
    // The regression this locks: `pm-focus` pulls in `--focus-ring`,
    // the 3px box-shadow meant for buttons. A text field drawing that
    // on top of its own border is the "heavy box" this pair replaced.
    const { container: i } = render(<Input value="" onChange={() => {}} />);
    expect(i.querySelector("input")!.className).not.toContain("pm-focus");

    const { container: t } = render(<Textarea value="" onChange={() => {}} />);
    expect(t.querySelector("textarea")!.className).not.toContain("pm-focus");
  });

  it("the wrapper draws no outline at rest and a flush accent outline on focus", () => {
    // Presence, not exact geometry — `fieldChrome.ts` is the one place
    // that owns the offset and width, and re-deriving its values here
    // would just be a second copy to keep in sync.
    const { container } = render(<Input value="" onChange={() => {}} />);
    const input = container.querySelector("input")!;
    const wrapper = input.parentElement!;
    expect(wrapper.style.outline).toBe("none");

    // `element.focus()` moves DOM focus but does not reliably reach
    // React's synthetic `onFocus` in jsdom; `fireEvent` does.
    fireEvent.focus(input);
    expect(wrapper.style.outline).toContain("solid");
    expect(wrapper.style.outline).not.toBe("none");
    // Not the button ring: no box-shadow appears on the wrapper.
    expect(wrapper.style.boxShadow).toBe("");
  });
});
