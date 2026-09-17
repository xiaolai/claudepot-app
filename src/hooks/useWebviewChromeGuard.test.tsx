import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useGlobalShortcuts } from "./useGlobalShortcuts";
import {
  isReloadCombo,
  nativeContextMenuWarranted,
  useWebviewChromeGuard,
} from "./useWebviewChromeGuard";

/** A cancelable keydown — jsdom's default is `cancelable: false`, and
 *  `preventDefault()` on one of those is a silent no-op, so a test
 *  built on the default would pass against a guard that does nothing. */
function press(
  key: string,
  mods: {
    metaKey?: boolean;
    ctrlKey?: boolean;
    shiftKey?: boolean;
    altKey?: boolean;
  } = {},
): KeyboardEvent {
  const e = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...mods,
  });
  window.dispatchEvent(e);
  return e;
}

function rightClick(el: Element): MouseEvent {
  const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(e);
  return e;
}

/** Put a real, non-collapsed selection over an element's text. */
function selectInside(el: Element): void {
  const range = document.createRange();
  range.selectNodeContents(el);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

beforeEach(() => {
  document.body.innerHTML = "";
  window.getSelection()?.removeAllRanges();
});

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("isReloadCombo", () => {
  it("matches every combination a webview reads as reload", () => {
    expect(isReloadCombo({ key: "F5", metaKey: false, ctrlKey: false,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(true);
    // Chromium's hard reloads.
    expect(isReloadCombo({ key: "F5", metaKey: false, ctrlKey: true,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(true);
    expect(isReloadCombo({ key: "r", metaKey: true, ctrlKey: false,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(true);
    expect(isReloadCombo({ key: "r", metaKey: false, ctrlKey: true,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(true);
    // Shift reports the letter uppercase.
    expect(isReloadCombo({ key: "R", metaKey: true, ctrlKey: false,
      shiftKey: true, altKey: false } as KeyboardEvent)).toBe(true);
    expect(isReloadCombo({ key: "R", metaKey: false, ctrlKey: true,
      shiftKey: true, altKey: false } as KeyboardEvent)).toBe(true);
  });

  it("leaves everything else alone", () => {
    // No modifier: typing an r.
    expect(isReloadCombo({ key: "r", metaKey: false, ctrlKey: false,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(false);
    // ⌥⌘R is not a reload on any platform.
    expect(isReloadCombo({ key: "r", metaKey: true, ctrlKey: false,
      shiftKey: false, altKey: true } as KeyboardEvent)).toBe(false);
    // Neighbouring app shortcuts and function keys.
    expect(isReloadCombo({ key: "k", metaKey: true, ctrlKey: false,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(false);
    expect(isReloadCombo({ key: "F6", metaKey: false, ctrlKey: false,
      shiftKey: false, altKey: false } as KeyboardEvent)).toBe(false);
  });
});

describe("nativeContextMenuWarranted", () => {
  it("keeps the native menu for editable fields", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    expect(nativeContextMenuWarranted(input, null)).toBe(true);
  });

  it("keeps it for the element holding a live selection", () => {
    document.body.innerHTML = "<p id='p'>a path worth copying</p>";
    const p = document.getElementById("p")!;
    selectInside(p);
    expect(nativeContextMenuWarranted(p, window.getSelection())).toBe(true);
  });

  it("does not keep it for chrome elsewhere while a selection stands", () => {
    // The hole this argument exists to close: a selection made in one
    // pane stays non-collapsed, so testing `isCollapsed` alone would
    // hand the webview's menu back for every later right-click.
    document.body.innerHTML =
      "<p id='p'>selected</p><div id='chrome'>chrome</div>";
    selectInside(document.getElementById("p")!);
    const chrome = document.getElementById("chrome")!;
    expect(nativeContextMenuWarranted(chrome, window.getSelection())).toBe(
      false,
    );
  });

  it("does not keep it for plain chrome with nothing selected", () => {
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(nativeContextMenuWarranted(div, window.getSelection())).toBe(false);
  });
});

describe("useWebviewChromeGuard, enabled", () => {
  it("cancels the reload keys", () => {
    renderHook(() => useWebviewChromeGuard(true));
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(true);
    expect(press("R", { metaKey: true, shiftKey: true }).defaultPrevented).toBe(
      true,
    );
    expect(press("F5").defaultPrevented).toBe(true);
  });

  it("cancels ⌘R even while an input has focus", () => {
    // Deliberately NOT behind isShortcutContextBlocked: a focused
    // field is where a reload costs the most.
    renderHook(() => useWebviewChromeGuard(true));
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(true);
  });

  it("still lets ⌘R refresh the section", () => {
    // preventDefault without stopPropagation — a guard that swallowed
    // the event would silently disable the documented shortcut.
    const onRefresh = vi.fn();
    renderHook(() => {
      useWebviewChromeGuard(true);
      useGlobalShortcuts({ onRefresh });
    });
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(true);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("sees the key even when a child stops propagation", () => {
    // Why the listener is in the capture phase: a modal or the palette
    // input that calls stopPropagation on its own keydown would
    // otherwise hide the reload key from a bubble-phase guard.
    renderHook(() => useWebviewChromeGuard(true));
    const div = document.createElement("div");
    document.body.appendChild(div);
    div.addEventListener("keydown", (e) => e.stopPropagation());
    const e = new KeyboardEvent("keydown", {
      key: "r",
      metaKey: true,
      bubbles: true,
      cancelable: true,
    });
    div.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
  });

  it("leaves other keys alone", () => {
    renderHook(() => useWebviewChromeGuard(true));
    expect(press("k", { metaKey: true }).defaultPrevented).toBe(false);
    expect(press("r").defaultPrevented).toBe(false);
  });

  it("suppresses the native context menu on chrome", () => {
    renderHook(() => useWebviewChromeGuard(true));
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(rightClick(div).defaultPrevented).toBe(true);
  });

  it("leaves the native context menu on a text field and on a selection", () => {
    renderHook(() => useWebviewChromeGuard(true));
    const input = document.createElement("input");
    document.body.appendChild(input);
    expect(rightClick(input).defaultPrevented).toBe(false);

    document.body.insertAdjacentHTML("beforeend", "<p id='p'>path</p>");
    const p = document.getElementById("p")!;
    selectInside(p);
    expect(rightClick(p).defaultPrevented).toBe(false);
  });

  it("removes both listeners on unmount", () => {
    const { unmount } = renderHook(() => useWebviewChromeGuard(true));
    unmount();
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(false);
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(rightClick(div).defaultPrevented).toBe(false);
  });
});

describe("useWebviewChromeGuard, disabled", () => {
  it("cancels nothing", () => {
    renderHook(() => useWebviewChromeGuard(false));
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(false);
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(rightClick(div).defaultPrevented).toBe(false);
  });
});

describe("the default gate", () => {
  it("is off in a dev build", () => {
    vi.stubEnv("DEV", true);
    renderHook(() => useWebviewChromeGuard());
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(false);
  });

  it("is on in a packaged build", () => {
    // The direction that ships. Without this the suite would only ever
    // exercise the dev value Vitest itself runs under.
    vi.stubEnv("DEV", false);
    renderHook(() => useWebviewChromeGuard());
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(true);
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(rightClick(div).defaultPrevented).toBe(true);
  });
});
