import { describe, expect, it, beforeEach } from "vitest";
import { act, renderHook } from "@testing-library/react";

import { useSidebarCollapsed } from "./useSidebarCollapsed";

describe("useSidebarCollapsed", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("defaults to expanded when no localStorage entry exists", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
  });

  it("bootstraps to collapsed when localStorage carries the marker", () => {
    localStorage.setItem("cp-sidebar-collapsed", "1");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
  });

  it("ignores non-marker values in localStorage", () => {
    // Defensive: localStorage is user-mutable; only the exact "1"
    // sentinel should collapse. "true" / "yes" / garbage should fall
    // back to expanded so corrupted state never wedges the user.
    localStorage.setItem("cp-sidebar-collapsed", "true");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
  });

  it("toggle flips the state and persists the marker", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
    act(() => result.current.toggle());
    expect(result.current.collapsed).toBe(true);
    expect(localStorage.getItem("cp-sidebar-collapsed")).toBe("1");
  });

  it("toggle back to expanded removes the marker (no stale '0')", () => {
    // We store the marker by absence, not by an explicit "0". A
    // stored "0" would be a regression — the bootstrap reader checks
    // === "1" only, so "0" would be treated as expanded too, but
    // leaving a non-marker value behind invites future drift.
    localStorage.setItem("cp-sidebar-collapsed", "1");
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => result.current.toggle());
    expect(result.current.collapsed).toBe(false);
    expect(localStorage.getItem("cp-sidebar-collapsed")).toBeNull();
  });

  it("setCollapsed accepts an explicit boolean", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => result.current.setCollapsed(true));
    expect(result.current.collapsed).toBe(true);
    act(() => result.current.setCollapsed(false));
    expect(result.current.collapsed).toBe(false);
  });

  it("⌘\\ keyboard event toggles the sidebar", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "\\", metaKey: true }),
      );
    });
    expect(result.current.collapsed).toBe(true);
  });

  it("does not fire while a text input is focused", () => {
    // Shell shortcuts must not steal keystrokes from the user typing
    // — e.g. an editor that accepts `\` as part of a regex search.
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "\\",
          metaKey: true,
          bubbles: true,
        }),
      );
    });
    expect(result.current.collapsed).toBe(false);
    document.body.removeChild(input);
  });

  it("plain backslash without modifier is ignored", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "\\" }));
    });
    expect(result.current.collapsed).toBe(false);
  });
});
