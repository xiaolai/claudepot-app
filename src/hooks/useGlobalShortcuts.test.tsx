import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  isEditable,
  isShortcutContextBlocked,
  useGlobalShortcuts,
} from "./useGlobalShortcuts";

function press(key: string, mods: { metaKey?: boolean; shiftKey?: boolean;
  altKey?: boolean; ctrlKey?: boolean } = {}) {
  window.dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, ...mods }),
  );
}

beforeEach(() => {
  document.body.innerHTML = "";
});

/**
 * This hook is the enforcement point for design.md → Shortcuts
 * ("never fire while a modal is open or an input is focused") for the
 * section-scoped keys. It had no direct coverage, so the gate could
 * regress silently in either direction.
 */
describe("useGlobalShortcuts", () => {
  it("fires ⌘R and ⌘N when nothing is blocking", () => {
    const onRefresh = vi.fn();
    const onAdd = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh, onAdd }));

    press("r", { metaKey: true });
    press("n", { metaKey: true });

    expect(onRefresh).toHaveBeenCalledTimes(1);
    expect(onAdd).toHaveBeenCalledTimes(1);
  });

  it("does nothing for a key with no handler passed", () => {
    const onRefresh = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh }));
    press("n", { metaKey: true });
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("ignores the keys without a modifier", () => {
    const onRefresh = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh }));
    press("r");
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("ignores the keys when Shift or Alt is held", () => {
    const onRefresh = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh }));
    press("r", { metaKey: true, shiftKey: true });
    press("r", { metaKey: true, altKey: true });
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("does not fire while an input is focused", () => {
    const onRefresh = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh }));
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    press("r", { metaKey: true });
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("does not fire while a modal is open", () => {
    const onRefresh = vi.fn();
    const onAdd = vi.fn();
    renderHook(() => useGlobalShortcuts({ onRefresh, onAdd }));
    document.body.innerHTML =
      '<div role="dialog" aria-modal="true"><button id="ok">ok</button></div>';
    document.getElementById("ok")?.focus();

    press("r", { metaKey: true });
    press("n", { metaKey: true });
    expect(onRefresh).not.toHaveBeenCalled();
    expect(onAdd).not.toHaveBeenCalled();
  });

  it("calls the latest handler after a re-render", () => {
    // The listener is installed once and reads callbacks from a ref;
    // a stale closure here would silently call the first render's
    // handler forever.
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = renderHook(
      ({ fn }) => useGlobalShortcuts({ onRefresh: fn }),
      { initialProps: { fn: first } },
    );

    rerender({ fn: second });
    press("r", { metaKey: true });

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });

  it("removes its listener on unmount", () => {
    const onRefresh = vi.fn();
    const { unmount } = renderHook(() => useGlobalShortcuts({ onRefresh }));
    unmount();
    press("r", { metaKey: true });
    expect(onRefresh).not.toHaveBeenCalled();
  });
});

describe("isEditable / isShortcutContextBlocked", () => {
  it("treats input, textarea and select as editable", () => {
    for (const tag of ["input", "textarea", "select"]) {
      expect(isEditable(document.createElement(tag))).toBe(true);
    }
    // The contentEditable branch is deliberately not asserted here:
    // jsdom does not implement `isContentEditable` (it stays false
    // however the attribute is set), so a passing test would only be
    // describing the test environment, not the browser behavior.
  });

  it("treats a plain element and null as not editable", () => {
    expect(isEditable(document.createElement("div"))).toBe(false);
    expect(isEditable(null)).toBe(false);
  });

  it("blocks whenever any dialog is in the DOM", () => {
    expect(isShortcutContextBlocked()).toBe(false);
    document.body.innerHTML = '<div role="dialog"></div>';
    expect(isShortcutContextBlocked()).toBe(true);
  });
});
