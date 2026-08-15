/**
 * The tab-forcing half of D3's deep link.
 *
 * `UpdatesPanel`'s "Edit in Env variables" only sets the sub-route.
 * That is enough because `GlobalSection` already forces the Config tab
 * for any `node:*` route — a fact worth a test, since the link would
 * otherwise land on whichever tab was last open and silently do
 * nothing, which is exactly the bug that rule was written for.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("../api", () => ({
  api: {
    configScan: vi.fn().mockResolvedValue({
      scopes: [],
      cwd: null,
      project_root: null,
      config_home_dir: null,
      memory_slug: "",
      memory_slug_lossy: false,
    }),
    configListEditors: vi.fn().mockResolvedValue([]),
    configGetEditorDefaults: vi
      .fn()
      .mockResolvedValue({ by_kind: {}, fallback: "system" }),
    configWatchStart: vi.fn().mockResolvedValue(undefined),
    configWatchStop: vi.fn().mockResolvedValue(undefined),
    configPreview: vi.fn(),
    projectList: vi.fn().mockResolvedValue([]),
    artifactUsageBatch: vi.fn().mockResolvedValue([]),
    artifactListDisabled: vi.fn().mockResolvedValue([]),
    ccEnvList: vi.fn().mockResolvedValue({
      documented: [],
      undocumented: { state: "available", snapshot_version: "2.1.220", names: [] },
      unrecognized: [],
      docs_fetched_at: "2026-07-28",
      docs_sha256: "a".repeat(64),
      binary_crosscheck_version: "2.1.220",
      installed_version: "2.1.220",
      installed_path: "/opt/claude",
      settings_path: "/home/u/.claude/settings.json",
      categories: [{ key: "misc", label: "Other" }],
      crosscheck_is_exact: true,
    }),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: vi.fn() }),
}));

import { GlobalSection } from "./GlobalSection";
import { GLOBAL_TABS } from "./global/tabs";
import {
  consumeGlobalTabHint,
  triggerGlobalTab,
} from "../lib/networkPanelDeepLink";

/**
 * The Files tab, found by its stable id rather than its label.
 *
 * These queried `name: /config/i`, which broke the moment the label
 * changed — the section is now "Config" and its first tab was renamed
 * to "Files" so the two would stop reading as the same thing. A test
 * that pins a translated label is asserting the copy, not the
 * behaviour.
 */
function configTab(): HTMLElement {
  const el = document.getElementById("global-tab-config");
  if (!el) throw new Error("global-tab-config is not rendered");
  return el;
}

describe("GlobalSection — env-vars deep link", () => {
  it("forces the Config tab and opens the pane for node:virtual:env-vars", async () => {
    // Start on Updates, which is where the link is clicked from.
    window.localStorage.setItem("claudepot.global.tab", "updates");

    const { rerender } = render(
      <GlobalSection subRoute={null} onSubRouteChange={() => {}} />,
    );
    rerender(
      <GlobalSection
        subRoute="node:virtual:env-vars"
        onSubRouteChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(configTab()).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
    expect(
      await screen.findByLabelText("Search environment variables"),
    ).toBeInTheDocument();
  });
});

/**
 * The ⌘K tab deep link rides a transient sessionStorage hint plus a
 * window event — NOT the section `subRoute`. `subRoute` is persisted
 * per section, so writing a tab id there both clobbered the stored
 * `node:<id>` Config route and outlived the single navigation it was
 * describing.
 */
describe("GlobalSection — ⌘K tab deep link", () => {
  it("selects the hinted tab on a cold mount", async () => {
    window.localStorage.setItem("claudepot.global.tab", "config");
    triggerGlobalTab("updates");

    render(<GlobalSection subRoute={null} onSubRouteChange={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByRole("tab", { name: /updates/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  });

  it("selects the tab when Global is already mounted", async () => {
    window.localStorage.setItem("claudepot.global.tab", "config");
    render(<GlobalSection subRoute={null} onSubRouteChange={vi.fn()} />);

    // setSection is a no-op when Global is already active, so only the
    // event path can reach a mounted section.
    triggerGlobalTab("memory");

    await waitFor(() => {
      expect(screen.getByRole("tab", { name: /memory/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  });

  it("consumes the hint so it fires exactly once", () => {
    triggerGlobalTab("tips");
    expect(consumeGlobalTabHint()).toBe("tips");
    expect(consumeGlobalTabHint()).toBeNull();
  });

  it("never touches the section sub-route", async () => {
    const onSubRouteChange = vi.fn();
    triggerGlobalTab("updates");
    render(
      <GlobalSection subRoute="node:virtual:env-vars" onSubRouteChange={onSubRouteChange} />,
    );
    await waitFor(() => {
      expect(configTab()).toBeInTheDocument();
    });
    // A node: route must survive a tab deep link untouched.
    expect(onSubRouteChange).not.toHaveBeenCalledWith(null);
  });

  it("ignores a hint naming a tab that does not exist", async () => {
    window.localStorage.setItem("claudepot.global.tab", "config");
    triggerGlobalTab("nonsense");
    render(<GlobalSection subRoute={null} onSubRouteChange={vi.fn()} />);
    await waitFor(() => {
      expect(configTab()).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  });

  it("wins over a Config node route restored from localStorage", async () => {
    // `node:<id>` doubles as the Config tree's PERSISTED selection, so
    // setSection("global") restores one on any later visit. Forcing
    // the Config tab on that restore made "Open Global → Updates" land
    // on Config for anyone who had ever clicked a Config node.
    window.localStorage.setItem("claudepot.global.tab", "config");
    triggerGlobalTab("updates");

    render(
      <GlobalSection
        subRoute="node:virtual:env-vars"
        onSubRouteChange={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("tab", { name: /updates/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  });

  it("renders one tab per GLOBAL_TABS entry, in that order", () => {
    render(<GlobalSection subRoute={null} onSubRouteChange={vi.fn()} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(GLOBAL_TABS.length);
    GLOBAL_TABS.forEach((t, i) => {
      expect(tabs[i]!.textContent).toContain(t.label);
    });
  });
});
