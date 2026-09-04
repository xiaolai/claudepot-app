import { describe, expect, it, beforeEach } from "vitest";
import { act, renderHook } from "@testing-library/react";

import { useSidebarCollapsed } from "./useSidebarCollapsed";

describe("useSidebarCollapsed", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("defaults to collapsed when no localStorage entry exists", () => {
    // The rail is the resting state: a fresh profile starts collapsed.
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
  });

  it("bootstraps to collapsed when localStorage carries '1'", () => {
    localStorage.setItem("cp-sidebar-collapsed", "1");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
  });

  it("bootstraps to expanded when localStorage carries '0'", () => {
    // Expanded is a stored choice, not the absence of one. Without this
    // a user who opened the sidebar would find it collapsed again on
    // every launch — the default would win over their decision.
    localStorage.setItem("cp-sidebar-collapsed", "0");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
  });

  it("treats non-sentinel values as never chosen", () => {
    // Defensive: localStorage is user-mutable; only the exact "1" /
    // "0" sentinels carry meaning. "true" / "yes" / garbage fall back
    // to the default rather than being guessed at.
    localStorage.setItem("cp-sidebar-collapsed", "true");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
  });

  it("toggle flips the state and persists an explicit '0' for expanded", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
    act(() => result.current.toggle());
    expect(result.current.collapsed).toBe(false);
    expect(localStorage.getItem("cp-sidebar-collapsed")).toBe("0");
  });

  it("toggle back to collapsed persists '1'", () => {
    localStorage.setItem("cp-sidebar-collapsed", "0");
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(false);
    act(() => result.current.toggle());
    expect(result.current.collapsed).toBe(true);
    expect(localStorage.getItem("cp-sidebar-collapsed")).toBe("1");
  });

  it("an expanded choice survives a remount", () => {
    // The regression the three-state encoding exists to prevent: open
    // the sidebar, come back later, and it is still open.
    const first = renderHook(() => useSidebarCollapsed());
    act(() => first.result.current.setCollapsed(false));
    first.unmount();
    const second = renderHook(() => useSidebarCollapsed());
    expect(second.result.current.collapsed).toBe(false);
  });

  it("setCollapsed accepts an explicit boolean", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => result.current.setCollapsed(false));
    expect(result.current.collapsed).toBe(false);
    act(() => result.current.setCollapsed(true));
    expect(result.current.collapsed).toBe(true);
  });

  it("⌘\\ keyboard event toggles the sidebar", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    expect(result.current.collapsed).toBe(true);
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "\\", metaKey: true }),
      );
    });
    expect(result.current.collapsed).toBe(false);
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
    expect(result.current.collapsed).toBe(true);
    document.body.removeChild(input);
  });

  it("plain backslash without modifier is ignored", () => {
    const { result } = renderHook(() => useSidebarCollapsed());
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "\\" }));
    });
    expect(result.current.collapsed).toBe(true);
  });
});
