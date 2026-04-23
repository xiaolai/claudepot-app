import { describe, expect, it } from "vitest";
import { act, render, renderHook } from "@testing-library/react";
import { useSection } from "./useSection";

const IDS = ["a", "b", "c", "d"] as const;

function fireCmdDigit(digit: string) {
  const ev = new KeyboardEvent("keydown", {
    key: digit,
    metaKey: true,
    bubbles: true,
    cancelable: true,
  });
  window.dispatchEvent(ev);
}

describe("useSection — ⌘1..⌘9 gating", () => {
  it("switches sections on ⌘N when no input is focused and no modal is open", () => {
    const { result } = renderHook(() => useSection(IDS[0], IDS));
    expect(result.current.section).toBe("a");
    act(() => fireCmdDigit("3"));
    expect(result.current.section).toBe("c");
  });

  it("does NOT switch when focus is inside a text input", () => {
    const { result } = renderHook(() => useSection(IDS[0], IDS));
    const { getByRole } = render(<input aria-label="q" />);
    act(() => getByRole("textbox").focus());
    act(() => fireCmdDigit("3"));
    expect(result.current.section).toBe("a");
  });

  it("does NOT switch when focus is inside a textarea", () => {
    const { result } = renderHook(() => useSection(IDS[0], IDS));
    const { getByRole } = render(<textarea aria-label="t" />);
    act(() => getByRole("textbox").focus());
    act(() => fireCmdDigit("2"));
    expect(result.current.section).toBe("a");
  });

  it("does NOT switch when focus is on a contentEditable element", () => {
    const { result } = renderHook(() => useSection(IDS[0], IDS));
    const { container } = render(
      <div contentEditable suppressContentEditableWarning data-testid="ce" />,
    );
    const el = container.querySelector<HTMLElement>('[data-testid="ce"]')!;
    act(() => el.focus());
    act(() => fireCmdDigit("4"));
    expect(result.current.section).toBe("a");
  });

  it("does NOT switch when a [role=dialog] element is present", () => {
    const { result } = renderHook(() => useSection(IDS[0], IDS));
    const dlg = document.createElement("div");
    dlg.setAttribute("role", "dialog");
    document.body.appendChild(dlg);
    try {
      act(() => fireCmdDigit("4"));
      expect(result.current.section).toBe("a");
    } finally {
      document.body.removeChild(dlg);
    }
  });
});
