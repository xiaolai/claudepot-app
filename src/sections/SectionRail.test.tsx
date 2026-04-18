// Tests for the rail + useSection wiring. Uses a local registry so
// the tests don't depend on AccountsSection's Tauri surface.

import React from "react";
import { describe, expect, it, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Icon } from "../components/Icon";
import { SectionRail } from "../components/SectionRail";
import { useSection } from "../hooks/useSection";
import type { SectionDef } from "./registry";

const fakeSections: SectionDef[] = [
  { id: "accounts", label: "Accounts", icon: <Icon name="user" size={18} /> },
  { id: "settings", label: "Settings", icon: <Icon name="settings" size={18} /> },
];

const bodies: Record<string, React.FC> = {
  accounts: () => <div>acc body</div>,
  settings: () => <div>settings body</div>,
};

function Harness() {
  const ids = fakeSections.map((s) => s.id);
  const { section, setSection } = useSection(ids[0], ids);
  const Active = bodies[section] ?? bodies.accounts;
  return (
    <>
      <SectionRail sections={fakeSections} active={section} onSelect={setSection} />
      <Active />
    </>
  );
}

beforeEach(() => {
  localStorage.clear();
});

describe("SectionRail + useSection", () => {
  it("renders one button per section with accessible labels", () => {
    render(<Harness />);
    expect(screen.getByRole("button", { name: "Accounts" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("clicking a rail button switches the active section", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    expect(screen.getByText("acc body")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByText("settings body")).toBeInTheDocument();
    expect(screen.queryByText("acc body")).not.toBeInTheDocument();
  });

  it("marks the active section with aria-current=page", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const accBtn = screen.getByRole("button", { name: "Accounts" });
    const setBtn = screen.getByRole("button", { name: "Settings" });
    expect(accBtn).toHaveAttribute("aria-current", "page");
    expect(setBtn).not.toHaveAttribute("aria-current");

    await user.click(setBtn);
    expect(setBtn).toHaveAttribute("aria-current", "page");
    expect(accBtn).not.toHaveAttribute("aria-current");
  });

  it("⌘2 shortcut activates the second section", () => {
    render(<Harness />);
    expect(screen.getByText("acc body")).toBeInTheDocument();

    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "2", metaKey: true, bubbles: true }),
      );
    });

    expect(screen.getByText("settings body")).toBeInTheDocument();
  });

  it("ignores shortcuts past the registered count", () => {
    render(<Harness />);
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "5", metaKey: true, bubbles: true }),
      );
    });
    // Still on the default section.
    expect(screen.getByText("acc body")).toBeInTheDocument();
  });

  it("persists the active section to localStorage", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(localStorage.getItem("claudepot.activeSection")).toBe("settings");
  });

  it("restores the last active section on remount", () => {
    localStorage.setItem("claudepot.activeSection", "settings");
    render(<Harness />);
    expect(screen.getByText("settings body")).toBeInTheDocument();
  });

  it("falls back to the default when localStorage holds a stale id", () => {
    localStorage.setItem("claudepot.activeSection", "nonexistent-old-section");
    render(<Harness />);
    expect(screen.getByText("acc body")).toBeInTheDocument();
  });

  it("does not switch on Ctrl+Shift+1 or other modifier combos", () => {
    render(<Harness />);
    act(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "2",
          metaKey: true,
          shiftKey: true,
          bubbles: true,
        }),
      );
    });
    expect(screen.getByText("acc body")).toBeInTheDocument();
  });
});
