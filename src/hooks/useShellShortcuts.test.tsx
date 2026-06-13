import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useShellShortcuts } from "./useShellShortcuts";

function press(
  key: string,
  mods: Partial<
    Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey" | "shiftKey">
  > = {},
) {
  window.dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, ...mods }),
  );
}

function renderShortcuts() {
  const args = {
    setSection: vi.fn(),
    openPalette: vi.fn(),
    openShortcuts: vi.fn(),
    pushToast: vi.fn(),
  };
  const utils = renderHook(() => useShellShortcuts(args));
  return { args, ...utils };
}

beforeEach(() => {
  try {
    localStorage.clear();
  } catch {
    /* ignore */
  }
  document.body.innerHTML = "";
});

describe("useShellShortcuts", () => {
  it("⌘, opens Settings", () => {
    const { args } = renderShortcuts();
    press(",", { metaKey: true });
    expect(args.setSection).toHaveBeenCalledWith("settings");
  });

  it("⌘K opens the palette", () => {
    const { args } = renderShortcuts();
    press("k", { metaKey: true });
    expect(args.openPalette).toHaveBeenCalledTimes(1);
  });

  it("⌘/ opens the shortcuts reference", () => {
    const { args } = renderShortcuts();
    press("/", { metaKey: true });
    expect(args.openShortcuts).toHaveBeenCalledTimes(1);
  });

  it("⌘K is ignored while an input is focused", () => {
    const { args } = renderShortcuts();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    press("k", { metaKey: true });
    expect(args.openPalette).not.toHaveBeenCalled();
  });

  it("plain K (no modifier) does nothing", () => {
    const { args } = renderShortcuts();
    press("k");
    expect(args.openPalette).not.toHaveBeenCalled();
  });

  it("⌃⌥⌘L toggles developer mode and toasts the new state", () => {
    const { args } = renderShortcuts();
    press("l", { metaKey: true, ctrlKey: true, altKey: true });
    expect(args.pushToast).toHaveBeenCalledWith("info", "Developer mode on");
    expect(localStorage.getItem("cp-dev-mode")).toBe("1");

    press("l", { metaKey: true, ctrlKey: true, altKey: true });
    expect(args.pushToast).toHaveBeenLastCalledWith(
      "info",
      "Developer mode off",
    );
    expect(localStorage.getItem("cp-dev-mode")).toBe("0");
  });

  it("⌘L without the full four-modifier combo does NOT toggle dev mode", () => {
    const { args } = renderShortcuts();
    press("l", { metaKey: true });
    expect(args.pushToast).not.toHaveBeenCalled();
  });

  it("⌘⇧L focuses the first live-strip row", () => {
    renderShortcuts();
    document.body.innerHTML = `
      <div aria-label="Live Claude sessions">
        <button role="option" id="row-1">one</button>
        <button role="option" id="row-2">two</button>
      </div>`;
    press("l", { metaKey: true, shiftKey: true });
    expect(document.activeElement?.id).toBe("row-1");
  });

  it("removes every listener on unmount", () => {
    const { args, unmount } = renderShortcuts();
    unmount();
    press(",", { metaKey: true });
    press("k", { metaKey: true });
    press("l", { metaKey: true, ctrlKey: true, altKey: true });
    expect(args.setSection).not.toHaveBeenCalled();
    expect(args.openPalette).not.toHaveBeenCalled();
    expect(args.pushToast).not.toHaveBeenCalled();
  });
});
