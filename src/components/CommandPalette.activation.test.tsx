import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

vi.mock("../api", () => ({
  api: {
    sessionSearch: vi.fn().mockResolvedValue([]),
    projectList: vi.fn().mockResolvedValue([]),
  },
}));

import { CommandPalette } from "./CommandPalette";
import { sampleStatus } from "../test/fixtures";

/**
 * The palette renders rows grouped by category (Quick Switch →
 * Navigate → Actions) while the action list is produced in a
 * different order. Any keyboard activation that resolves the cursor
 * against the *production* order rather than the *visible* order runs
 * a command the user never pointed at — the original bug fired
 * "Sign Desktop out" when "Open Projects" was highlighted.
 *
 * These tests pin the invariant that matters regardless of how the
 * ordering is implemented: whatever row is marked `aria-selected`,
 * Enter runs THAT row.
 */
function renderPalette(overrides: Record<string, unknown> = {}) {
  const handlers = {
    onClose: vi.fn(),
    onSwitchCli: vi.fn(),
    onSwitchDesktop: vi.fn(),
    onAdd: vi.fn(),
    onRefresh: vi.fn(),
    onRemove: vi.fn(),
    onClearDesktop: vi.fn(),
    onLaunchDesktop: vi.fn(),
    onNavigate: vi.fn(),
    onShowShortcuts: vi.fn(),
  };
  render(
    <CommandPalette
      accounts={[]}
      status={sampleStatus()}
      {...handlers}
      {...overrides}
    />,
  );
  return handlers;
}

/** The row the user currently sees highlighted. */
function selectedRowText(): string {
  const rows = screen.getAllByRole("option");
  const sel = rows.find((r) => r.getAttribute("aria-selected") === "true");
  if (!sel) throw new Error("no row is aria-selected");
  return sel.textContent ?? "";
}

function input(): HTMLElement {
  return screen.getByPlaceholderText(/Search/i);
}

describe("CommandPalette — keyboard activation targets the visible row", () => {
  // Pin the optional-section state. Boards is the optional tenth
  // section, off by default and persisted in localStorage — so the
  // palette renders nine rows or ten depending on what a previous test
  // in this file left behind. That is the real cause of the
  // intermittent `expected 9 to be 10`: the row set moved underneath
  // the test, and no amount of flushing async renders touches it.
  //
  // Clearing rather than setting "0": the module also keeps an
  // in-memory mirror for when storage is unavailable, and an absent key
  // exercises the same default-off path the app boots into.
  beforeEach(() => {
    localStorage.clear();
  });

  it("Enter on the initially-selected row runs that row, not another category's", () => {
    const h = renderPalette();

    // Whatever lands first, it must not be a destructive Desktop action
    // firing from a navigation row.
    const label = selectedRowText();
    fireEvent.keyDown(input(), { key: "Enter" });

    if (label.startsWith("Open Projects")) {
      expect(h.onNavigate).toHaveBeenCalledWith("projects", undefined);
      expect(h.onClearDesktop).not.toHaveBeenCalled();
    }
    // The destructive action must never fire unless it was the row shown.
    if (h.onClearDesktop.mock.calls.length > 0) {
      expect(label).toMatch(/Sign Desktop out/);
    }
  });

  it("Enter after ArrowDown runs the row that ArrowDown highlighted", () => {
    const h = renderPalette();

    fireEvent.keyDown(input(), { key: "ArrowDown" });
    const label = selectedRowText();
    fireEvent.keyDown(input(), { key: "Enter" });

    if (label.startsWith("Sign Desktop out")) {
      expect(h.onClearDesktop).toHaveBeenCalledTimes(1);
    } else {
      expect(h.onClearDesktop).not.toHaveBeenCalled();
    }
  });

  it("keeps rows out of the tab order so focus can't desync from the cursor", () => {
    // Rows were <button>s. Tabbing to one and pressing Enter fired
    // BOTH the row's own click and the dialog's keydown handler
    // resolving selectable[selectedIndex] — two activations, one of
    // them a command the user never pointed at. The combobox pattern
    // keeps focus in the input; rows are pointer targets only.
    renderPalette();
    for (const row of screen.getAllByRole("option")) {
      expect(row.tagName).toBe("LI");
      expect(row).not.toHaveAttribute("tabindex");
    }
    // The input is the only tab stop inside the dialog.
    const focusable = document
      .querySelector('[role="dialog"]')!
      .querySelectorAll('a[href], button, input, select, textarea, [tabindex]');
    const tabbable = Array.from(focusable).filter(
      (el) => el.getAttribute("tabindex") !== "-1",
    );
    expect(tabbable).toHaveLength(1);
    expect(tabbable[0]!.tagName).toBe("INPUT");
  });

  // `api.projectList` / `api.sessionSearch` are mocked async, so the
  // palette re-renders when they resolve. Without flushing, a
  // resolution can land between capturing the row list and asserting
  // against it — the row set shifts underneath the test and it fails
  // intermittently, which is exactly what CI saw on 2026-08-12.
  //
  // Flushing after every render makes the row set settled by
  // construction rather than by luck.
  const settle = async () => {
    await act(async () => {
      await Promise.resolve();
    });
  };

  it("every visible row activates the same handler by Enter as by click", async () => {
    // Walk the whole list with ArrowDown; at each stop, the keyboard
    // path and the click path must agree. This is the invariant the
    // original bug broke: clicking a row was correct, pressing Enter
    // on the same row was not.
    renderPalette();
    await settle();
    const rowsSeen = screen.getAllByRole("option").map((r) => r.textContent ?? "");
    cleanup();
    expect(rowsSeen.length).toBeGreaterThan(3);

    for (let target = 0; target < rowsSeen.length; target++) {
      const viaKeyboard = renderPalette();
      await settle();
      for (let i = 0; i < target; i++) {
        fireEvent.keyDown(input(), { key: "ArrowDown" });
      }
      const highlighted = selectedRowText();
      fireEvent.keyDown(input(), { key: "Enter" });
      const keyboardCalls = callSignature(viaKeyboard);
      cleanup();

      const viaClick = renderPalette();
      await settle();
      const row = screen
        .getAllByRole("option")
        .find((r) => (r.textContent ?? "") === highlighted);
      expect(row, `row "${highlighted}" not found for click`).toBeTruthy();
      fireEvent.click(row!);
      const clickCalls = callSignature(viaClick);
      cleanup();

      expect(keyboardCalls, `row ${target} "${highlighted}"`).toEqual(
        clickCalls,
      );
    }
  });
});

/** Which handlers fired, and how many times — order-independent. */
function callSignature(h: Record<string, { mock: { calls: unknown[][] } }>) {
  const out: Record<string, number> = {};
  for (const [name, fn] of Object.entries(h)) {
    if (name === "onClose") continue; // fires for every activation
    if (fn.mock.calls.length > 0) out[name] = fn.mock.calls.length;
  }
  return out;
}
