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

  // design.md → Shortcuts: "Never fire while a modal is open or an
  // input is focused." ⌘K used to check only the second half, so it
  // stacked the palette on top of an open dialog — two focus traps
  // fighting over the same tab order.
  it("none of ⌘K / ⌘, / ⌘/ fires while a modal is open", () => {
    const { args } = renderShortcuts();
    document.body.innerHTML = `
      <div role="dialog" aria-modal="true">
        <button id="ok">ok</button>
      </div>`;
    document.getElementById("ok")?.focus();

    press("k", { metaKey: true });
    press(",", { metaKey: true });
    press("/", { metaKey: true });

    expect(args.openPalette).not.toHaveBeenCalled();
    expect(args.setSection).not.toHaveBeenCalled();
    expect(args.openShortcuts).not.toHaveBeenCalled();
  });

  it("⌘, is ignored while an input is focused", () => {
    const { args } = renderShortcuts();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    press(",", { metaKey: true });
    expect(args.setSection).not.toHaveBeenCalled();
  });

  it("⌘⇧L is ignored while a modal is open", () => {
    renderShortcuts();
    document.body.innerHTML = `
      <div role="dialog" aria-modal="true"><button id="ok">ok</button></div>
      <div data-live-strip aria-label="Live Claude sessions">
        <button role="option" id="row-1">one</button>
      </div>`;
    const ok = document.getElementById("ok")!;
    ok.focus();
    press("l", { metaKey: true, shiftKey: true });
    expect(document.activeElement?.id).toBe("ok");
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
      <div data-live-strip aria-label="Live Claude sessions">
        <button role="option" id="row-1">one</button>
        <button role="option" id="row-2">two</button>
      </div>`;
    press("l", { metaKey: true, shiftKey: true });
    expect(document.activeElement?.id).toBe("row-1");
  });

  // The strip's aria-label is translated. This test used to build its
  // fixture with the English label and the hook used to select on that
  // label, so the pair agreed in English and agreed nowhere else —
  // ⌘⇧L was dead in zh-CN with the suite fully green. The label here is
  // deliberately Chinese: the shortcut must find the strip by its
  // stable `data-live-strip` hook, never by rendered copy.
  it("⌘⇧L finds the strip regardless of the translated aria-label", () => {
    renderShortcuts();
    document.body.innerHTML = `
      <div data-live-strip aria-label="实时 Claude 会话">
        <button role="option" id="row-1">one</button>
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
