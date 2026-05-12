import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { CopyButton } from "./CopyButton";

describe("CopyButton", () => {
  let writeTextSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    // jsdom defines `navigator.clipboard` as a non-writable getter so
    // a direct `vi.fn` assignment to `writeText` fails. `vi.stubGlobal`
    // would also miss it. Replace the property descriptor outright.
    writeTextSpy = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: writeTextSpy },
    });
  });

  it("renders nothing when text is empty — never overwrites the clipboard", async () => {
    // Session-detail bubbles render a copy button per row. An empty
    // user message or a malformed event has nothing to copy; clicking
    // would otherwise wipe whatever the user already had on the
    // clipboard with "". Guard centrally.
    const { container } = render(<CopyButton text="" />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders the button and copies text when non-empty", async () => {
    // user-event v14+ installs its own clipboard polyfill on
    // `userEvent.setup()`, which shadows the property we just
    // installed. Use fireEvent to dispatch a plain click instead so
    // the production code's `navigator.clipboard.writeText` actually
    // hits our spy.
    render(<CopyButton text="hello world" ariaLabel="Copy greeting" />);
    const btn = screen.getByRole("button", { name: /copy greeting/i });
    fireEvent.click(btn);
    await waitFor(() =>
      expect(writeTextSpy).toHaveBeenCalledWith("hello world"),
    );
  });

  it("uses the explicit ariaLabel rather than reading the full text", () => {
    // Prose surfaces (4000-char message bodies) must NOT use the
    // default `Copy ${text}` aria-label — a screen reader would
    // read out the whole body. Callers pass a short label instead.
    const longText = "A".repeat(4000);
    render(<CopyButton text={longText} ariaLabel="Copy assistant message" />);
    const btn = screen.getByRole("button", { name: "Copy assistant message" });
    expect(btn).toBeInTheDocument();
  });

  it("falls back to `Copy ${text}` when ariaLabel is omitted", () => {
    // Backward-compat for the 8 pre-existing call sites that pass
    // short identifiers (paths, UUIDs, URLs). Removing this default
    // would silently break their accessible names.
    render(<CopyButton text="/Users/me/proj" />);
    expect(
      screen.getByRole("button", { name: "Copy /Users/me/proj" }),
    ).toBeInTheDocument();
  });
});
