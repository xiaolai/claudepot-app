import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// `vi.mock` factories are hoisted above the module body, so the spies
// have to be created inside `vi.hoisted` to exist by then.
const { sessionSearch, projectList } = vi.hoisted(() => ({
  sessionSearch: vi.fn(),
  projectList: vi.fn(),
}));
vi.mock("../api", () => ({ api: { sessionSearch, projectList } }));

import { CommandPalette } from "./CommandPalette";
import { sampleStatus } from "../test/fixtures";
import {
  enabledSections,
  setSectionEnabled,
} from "../lib/optionalSections";
import { SETTINGS_PANES } from "../sections/settings/panes";
import { GLOBAL_TABS } from "../sections/global/tabs";
import { __resetProjectCache } from "../hooks/useProjectSearch";
import {
  DEEPLINK_GLOBAL_TAB_KEY,
  DEEPLINK_SETTINGS_TAB_KEY,
} from "../lib/storageKeys";
import { i18n } from "../lib/i18n";

// English section labels — tests run with the en locale active, and
// the palette rows are asserted against the English catalog.
const enShellT = i18n.getFixedT("en", "shell");

// Every `userEvent.setup()` here passes `{ delay: null }`. The coverage
// tests type a full label once per section, per Settings pane, and per
// Global tab, and each keystroke re-renders the whole row list — which
// now resolves a translation per row. With the default inter-keystroke
// delay the "exposes every Settings pane and Global tab" case measured
// 14.9 s against the suite's 15 s timeout, so it failed under any load
// and passed alone, which reads as flakiness rather than as the cost it
// is. Dropping the delay cuts it to ~10 s and changes timing only, not
// semantics. Per `vitest.config.ts`: make the test cheaper, don't raise
// the timeout.

function renderPalette(over: Record<string, unknown> = {}) {
  const h = {
    onClose: vi.fn(),
    onSwitchCli: vi.fn(),
    onSwitchDesktop: vi.fn(),
    onAdd: vi.fn(),
    onRefresh: vi.fn(),
    onRemove: vi.fn(),
    onNavigate: vi.fn(),
    onShowShortcuts: vi.fn(),
    onToggleTheme: vi.fn(),
  };
  render(
    <CommandPalette
      accounts={[]}
      status={sampleStatus()}
      {...h}
      {...over}
    />,
  );
  return h;
}

const input = () => screen.getByRole("combobox");
const rowTexts = () =>
  screen.queryAllByRole("option").map((r) => r.textContent ?? "");

beforeEach(() => {
  vi.clearAllMocks();
  sessionSearch.mockResolvedValue([]);
  projectList.mockResolvedValue([]);
  __resetProjectCache();
});

describe("CommandPalette — section coverage", () => {
  it("can reach every ENABLED section", async () => {
    // Six of nine sections used to be unreachable: the nav entries
    // were three hardcoded strings rather than the registry.
    for (const section of enabledSections()) {
      const label = enShellT(section.labelKey);
      const h = renderPalette();
      // One change event, not per-character typing — see the note on
      // the pane/tab coverage test below. The click below is still a
      // real event, so activation is exercised for every section.
      fireEvent.change(input(), { target: { value: label } });
      const row = screen
        .getAllByRole("option")
        .find((r) => r.textContent?.startsWith(`Open ${label}`));
      expect(row, `no palette row opens "${label}"`).toBeTruthy();
      fireEvent.click(row!);
      expect(h.onNavigate).toHaveBeenCalledWith(section.id);
      cleanup();
    }
  });

  it("cannot reach a section the user has switched off", async () => {
    // The whole point of one filtered list: a hidden section must be
    // unreachable, not merely absent from the sidebar. Offering it here
    // would make it invisible AND navigable, which is worse than either.
    setSectionEnabled("boards", false);
    renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "Boards");
    const row = screen
      .queryAllByRole("option")
      .find((r) => r.textContent?.startsWith("Open Boards"));
    expect(row, "a disabled section is reachable from ⌘K").toBeFalsy();
  });

  it("ranks the exact section above a scattered subsequence match", async () => {
    renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "keys");
    const first = rowTexts()[0] ?? "";
    expect(first).toContain("Keys");
  });
});

describe("CommandPalette — deep targets", () => {
  it("hides Settings panes and Global tabs until the user types", () => {
    renderPalette();
    const texts = rowTexts();
    expect(texts.some((t) => t.startsWith("Open Settings →"))).toBe(false);
    expect(texts.some((t) => t.startsWith("Open Global →"))).toBe(false);
    // Top-level sections are still there on an empty query.
    expect(texts.some((t) => t === "Open Settings")).toBe(true);
  });

  it("reaches a Settings pane and lands on the right tab", async () => {
    const h = renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "retention");

    const row = screen
      .getAllByRole("option")
      .find((r) => r.textContent?.includes("Settings → Retention"));
    expect(row).toBeTruthy();
    fireEvent.click(row!);

    expect(h.onNavigate).toHaveBeenCalledWith("settings");
    // The pane hint must be set for both the cold-mount and hot-mount
    // consumers in SettingsSection.
    expect(sessionStorage.getItem(DEEPLINK_SETTINGS_TAB_KEY)).toBe(
      "retention",
    );
  });

  it("finds a pane by keyword, not just by its label", async () => {
    renderPalette();
    const user = userEvent.setup({ delay: null });
    // "cleanupPeriodDays" is the CC setting Retention edits; the label
    // alone would never match what a user searching for it types.
    await user.type(input(), "cleanupPeriod");
    expect(
      rowTexts().some((t) => t.includes("Settings → Retention")),
    ).toBe(true);
  });

  it("reaches a Global tab via the transient tab hint", async () => {
    const h = renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "updates");

    const row = screen
      .getAllByRole("option")
      .find((r) => r.textContent?.includes("Global → Updates"));
    expect(row).toBeTruthy();
    fireEvent.click(row!);
    expect(h.onNavigate).toHaveBeenCalledWith("global");
    expect(sessionStorage.getItem(DEEPLINK_GLOBAL_TAB_KEY)).toBe("updates");
  });

  // Reachability, not typing mechanics: this asserts that a query
  // naming each pane/tab surfaces its row. Setting the query in one
  // change event tests exactly that, where `user.type` re-rendered the
  // whole row list once per character — 15 panes plus every Global tab,
  // each row resolving a translation. The keystroke-level behavior of
  // the input is covered by the interaction tests below, which still
  // use `userEvent`.
  it("exposes every Settings pane and Global tab as a target", () => {
    for (const pane of SETTINGS_PANES) {
      renderPalette();
      fireEvent.change(input(), {
        target: { value: `Settings ${pane.label}` },
      });
      const row = screen
        .queryAllByRole("option")
        .find((r) => r.textContent?.includes(`Settings → ${pane.label}`));
      expect(row, `pane "${pane.label}" unreachable`).toBeTruthy();
      cleanup();
    }
    for (const tab of GLOBAL_TABS) {
      renderPalette();
      fireEvent.change(input(), { target: { value: `Global ${tab.label}` } });
      const row = screen
        .queryAllByRole("option")
        .find((r) => r.textContent?.includes(`Global → ${tab.label}`));
      expect(row, `tab "${tab.label}" unreachable`).toBeTruthy();
      cleanup();
    }
  });
});

describe("CommandPalette — project search", () => {
  const project = {
    sanitized_name: "-Users-joker-github-claudepot",
    original_path: "/Users/joker/github/claudepot",
    session_count: 12,
    memory_file_count: 1,
    total_size_bytes: 1024,
    last_modified_ms: 1,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
  };

  it("does not touch the filesystem until the user types", () => {
    renderPalette();
    expect(projectList).not.toHaveBeenCalled();
  });

  it("finds a project by basename and opens it", async () => {
    projectList.mockResolvedValue([project]);
    renderPalette();
    const user = userEvent.setup({ delay: null });

    const dispatched: CustomEvent[] = [];
    const listener = (e: Event) => dispatched.push(e as CustomEvent);
    window.addEventListener("claudepot:navigate-section", listener);

    await user.type(input(), "claudepot");
    const row = await screen.findByRole("option", { name: /claudepot/ });
    fireEvent.click(row);

    window.removeEventListener("claudepot:navigate-section", listener);
    expect(dispatched).toHaveLength(1);
    expect(dispatched[0]!.detail).toEqual({
      id: "projects",
      projectPath: "/Users/joker/github/claudepot",
    });
  });

  it("shows the full path on hover — the row only has room for the basename", async () => {
    projectList.mockResolvedValue([project]);
    renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "claudepot");
    const row = await screen.findByRole("option", { name: /claudepot/ });
    expect(row.getAttribute("title")).toBe("/Users/joker/github/claudepot");
  });

  it("degrades to an empty state when the project list fails", async () => {
    projectList.mockRejectedValue(new Error("boom"));
    renderPalette();
    const user = userEvent.setup({ delay: null });
    await user.type(input(), "claudepot");
    // A failed project list must not blank the palette or throw — it
    // resolves to "no matches", the same as a query nothing matched.
    await waitFor(() => {
      expect(screen.getByText("No matches")).toBeInTheDocument();
    });
    expect(screen.queryByText(/…searching/)).not.toBeInTheDocument();
  });
});

describe("CommandPalette — accessibility", () => {
  it("gives the input combobox semantics pointing at the listbox", () => {
    renderPalette();
    const box = input();
    expect(box).toHaveAttribute("aria-autocomplete", "list");
    const listboxId = box.getAttribute("aria-controls");
    expect(listboxId).toBeTruthy();
    expect(document.getElementById(listboxId!)).toHaveAttribute(
      "role",
      "listbox",
    );
  });

  it("moves aria-activedescendant to the selected row as the user arrows", () => {
    renderPalette();
    const box = input();
    const rows = screen.getAllByRole("option");

    expect(box.getAttribute("aria-activedescendant")).toBe(rows[0]!.id);
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box.getAttribute("aria-activedescendant")).toBe(rows[1]!.id);
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(box.getAttribute("aria-activedescendant")).toBe(rows[0]!.id);
  });

  it("Home and End jump to the ends of the list", () => {
    renderPalette();
    const box = input();
    const rows = screen.getAllByRole("option");

    fireEvent.keyDown(box, { key: "End" });
    expect(box.getAttribute("aria-activedescendant")).toBe(
      rows[rows.length - 1]!.id,
    );
    fireEvent.keyDown(box, { key: "Home" });
    expect(box.getAttribute("aria-activedescendant")).toBe(rows[0]!.id);
  });

  it("renders no nested interactive elements", () => {
    renderPalette();
    // The dismiss scrim used to be an ancestor <button> wrapping the
    // dialog, which nests every option button inside a button.
    for (const btn of Array.from(document.querySelectorAll("button"))) {
      expect(btn.querySelector("button")).toBeNull();
    }
  });
});
