// `useSection` owns section state, including the case that arrived
// with optional sections: `ids` can now SHRINK at runtime.
//
// Before reconciliation lived here, hiding the active section left the
// app rendering a pane that was gone from the sidebar, the palette, and
// the ⌘ bindings — visible but unreachable. The one caller that handled
// it did so by reading localStorage, which could disagree with React's
// actual state. Centralizing it means a new way to hide a section
// inherits the fix instead of needing its own copy.

import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useSection } from "./useSection";

const ALL: readonly string[] = ["accounts", "boards", "settings"];
const WITHOUT_BOARDS: readonly string[] = ["accounts", "settings"];

beforeEach(() => localStorage.clear());
afterEach(() => localStorage.clear());

describe("useSection — reconciliation when ids shrink", () => {
  it("falls back to the default when the active section disappears", () => {
    const { result, rerender } = renderHook(
      ({ ids }) => useSection("accounts", ids),
      { initialProps: { ids: ALL } },
    );

    act(() => result.current.setSection("boards"));
    expect(result.current.section).toBe("boards");

    // The user switches Boards off.
    rerender({ ids: WITHOUT_BOARDS });

    expect(result.current.section).toBe("accounts");
  });

  it("persists the fallback so a reload does not resurrect the hidden section", () => {
    const { result, rerender } = renderHook(
      ({ ids }) => useSection("accounts", ids),
      { initialProps: { ids: ALL } },
    );
    act(() => result.current.setSection("boards"));
    rerender({ ids: WITHOUT_BOARDS });

    // Without this the stored id still reads "boards", and the next
    // launch would try to restore a section that no longer exists.
    expect(localStorage.getItem("claudepot.activeSection")).toBe("accounts");
  });

  it("leaves an active section alone when it is still enabled", () => {
    const { result, rerender } = renderHook(
      ({ ids }) => useSection("accounts", ids),
      { initialProps: { ids: ALL } },
    );
    act(() => result.current.setSection("settings"));
    // Boards disappears, but Settings is unaffected and must not move.
    rerender({ ids: WITHOUT_BOARDS });
    expect(result.current.section).toBe("settings");
  });

  it("a navigation back to the default still cancels the startup restore", () => {
    // The guard used to be `section !== defaultId`, which cannot tell
    // "never navigated" from "navigated away and back". A saved
    // section then overwrote a deliberate return to the default.
    localStorage.setItem("claudepot.activeSection", "settings");
    const { result } = renderHook(() => useSection("accounts", ALL));

    act(() => result.current.setSection("settings"));
    act(() => result.current.setSection("accounts"));

    expect(result.current.section).toBe("accounts");
  });

  it("re-enabling a section does not steal focus back", () => {
    // Toggling on should not yank the user out of wherever they are —
    // navigation on enable is the shortcut handler's decision, not a
    // side effect of the id list growing.
    const { result, rerender } = renderHook(
      ({ ids }) => useSection("accounts", ids),
      { initialProps: { ids: WITHOUT_BOARDS } },
    );
    act(() => result.current.setSection("settings"));
    rerender({ ids: ALL });
    expect(result.current.section).toBe("settings");
  });
});
