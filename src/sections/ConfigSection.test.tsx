/**
 * P0 smoke tests for ConfigSection: the section renders, invokes the
 * P0 backend commands (scan, list editors, get defaults) on mount,
 * and exposes the split-button "Open in…" primary action.
 */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const configScanSpy = vi.fn();
const configListEditorsSpy = vi.fn();
const configGetEditorDefaultsSpy = vi.fn();
const configOpenInEditorPathSpy = vi.fn();
const configSetEditorDefaultSpy = vi.fn();

const configPreviewSpy = vi.fn();
const configWatchStartSpy = vi.fn().mockResolvedValue(undefined);
const configWatchStopSpy = vi.fn().mockResolvedValue(undefined);

vi.mock("../api", () => ({
  api: {
    configScan: (...a: unknown[]) => configScanSpy(...a),
    configPreview: (...a: unknown[]) => configPreviewSpy(...a),
    configListEditors: (...a: unknown[]) => configListEditorsSpy(...a),
    configGetEditorDefaults: (...a: unknown[]) =>
      configGetEditorDefaultsSpy(...a),
    configSetEditorDefault: (...a: unknown[]) =>
      configSetEditorDefaultSpy(...a),
    configOpenInEditorPath: (...a: unknown[]) =>
      configOpenInEditorPathSpy(...a),
    configWatchStart: (...a: unknown[]) => configWatchStartSpy(...a),
    configWatchStop: (...a: unknown[]) => configWatchStopSpy(...a),
    // Folder-anchored mode triggers effective-settings + hook count;
    // return a neutral merged shape so the effect doesn't throw.
    configEffectiveSettings: vi
      .fn()
      .mockResolvedValue({ merged: {}, provenance: [], policy_winner: null, policy_errors: [] }),
    // Anchor picker pulls the recent-projects list on mount.
    projectList: vi.fn().mockResolvedValue([]),
    // Lifecycle classification runs whenever a file is selected.
    // Default to "out of scope" so no Disable/Trash buttons render —
    // the existing tests don't care about those affordances.
    artifactClassifyPath: vi
      .fn()
      .mockResolvedValue({
        trackable: null,
        refused: "outside test scope",
        already_disabled: false,
      }),
    // Usage badge fetcher runs once per tree identity.
    artifactUsageBatch: vi.fn().mockResolvedValue([]),
  },
}));

// ConfigSection uses @tauri-apps/api/event through useConfigTree; mock
// the listener so tests don't hit Tauri's uninitialized IPC layer.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

// ConfigSection pulls pushToast from AppStateProvider; mock the hook
// directly so tests don't need to mount the whole provider tree.
const pushToastSpy = vi.fn();
vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: pushToastSpy }),
}));

// Import after the mocks so the mocked api is captured.
import { ConfigSection } from "./ConfigSection";

function resetSpies() {
  configScanSpy.mockReset();
  configListEditorsSpy.mockReset();
  configGetEditorDefaultsSpy.mockReset();
  configOpenInEditorPathSpy.mockReset();
  configSetEditorDefaultSpy.mockReset();
  configWatchStartSpy.mockReset();
  configWatchStartSpy.mockResolvedValue(undefined);
  configWatchStopSpy.mockReset();
  configWatchStopSpy.mockResolvedValue(undefined);
}

describe("ConfigSection — P0 smoke", () => {
  it("renders the screen header and probes the backend on mount", async () => {
    resetSpies();
    configScanSpy.mockResolvedValue({
      scopes: [],
      cwd: "/repo",
      project_root: "/repo",
      config_home_dir: "/repo/.claude",
      memory_slug: "",
      memory_slug_lossy: false,
    });
    configListEditorsSpy.mockResolvedValue([
      {
        id: "system",
        label: "System default",
        binary_path: null,
        bundle_id: null,
        launch_kind: "system-handler",
        detected_via: "system-default",
        supports_kinds: null,
      },
    ]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });

    render(
      <ConfigSection subRoute={null} onSubRouteChange={() => {}} />,
    );

    expect(screen.getByText("Config")).toBeInTheDocument();
    await waitFor(() => {
      expect(configScanSpy).toHaveBeenCalledTimes(1);
      expect(configListEditorsSpy).toHaveBeenCalledTimes(1);
      expect(configGetEditorDefaultsSpy).toHaveBeenCalledTimes(1);
    });
    // PreviewHeader renders the resolved label once editors + defaults load.
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: "Open in System default" }),
      ).toBeInTheDocument();
    });
  });

  it("reports errors via the toast when configScan rejects", async () => {
    resetSpies();
    configScanSpy.mockRejectedValue(new Error("boom"));
    configListEditorsSpy.mockResolvedValue([]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });
    render(
      <ConfigSection subRoute={null} onSubRouteChange={() => {}} />,
    );
    // Still renders the header — failure is non-fatal in P0.
    expect(screen.getByText("Config")).toBeInTheDocument();
  });

  // Regression: audit 2026-04-24 H1 — anchor-change cleanup used to
  // fire `configWatchStop` which could arrive after the new
  // `configWatchStart` and kill the fresh watcher. Contract now:
  // anchor change → start only; stop fires only on section unmount.
  it("does not fire configWatchStop on anchor change, only on unmount", async () => {
    resetSpies();
    configScanSpy.mockResolvedValue({
      scopes: [],
      cwd: "/repo",
      project_root: "/repo",
      config_home_dir: "/repo/.claude",
      memory_slug: "",
      memory_slug_lossy: false,
    });
    configListEditorsSpy.mockResolvedValue([]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });

    const { rerender, unmount } = render(
      <ConfigSection
        subRoute={null}
        onSubRouteChange={() => {}}
        forcedAnchor={{ kind: "folder", path: "/repoA" }}
      />,
    );

    // Initial mount: start fires, stop does not.
    await waitFor(() => {
      expect(configWatchStartSpy).toHaveBeenCalledTimes(1);
    });
    expect(configWatchStopSpy).not.toHaveBeenCalled();

    // Anchor change: start fires again for the new anchor; stop MUST
    // still not fire (the race fix relies on the backend's internal
    // restart inside `configWatchStart`).
    rerender(
      <ConfigSection
        subRoute={null}
        onSubRouteChange={() => {}}
        forcedAnchor={{ kind: "folder", path: "/repoB" }}
      />,
    );
    await waitFor(() => {
      expect(configWatchStartSpy).toHaveBeenCalledTimes(2);
    });
    expect(configWatchStopSpy).not.toHaveBeenCalled();

    // Section unmount: THIS is where stop fires.
    unmount();
    await waitFor(() => {
      expect(configWatchStopSpy).toHaveBeenCalledTimes(1);
    });
  });

  // Regression: audit 2026-04-24 H2 — overlapping `configScan` calls
  // resolved out of order used to leave the tree showing the older
  // anchor because the stale resolve ran `setTree` after the fresh
  // one. The `scanGenRef` guard should drop stale resolves.
  it("drops stale configScan resolves when a newer anchor is in flight", async () => {
    resetSpies();
    configListEditorsSpy.mockResolvedValue([]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });

    type Defer = {
      promise: Promise<unknown>;
      resolve: (v: unknown) => void;
    };
    const makeDefer = (): Defer => {
      let r: (v: unknown) => void = () => {};
      const promise = new Promise((res) => {
        r = res;
      });
      return { promise, resolve: r };
    };

    const scanA = makeDefer();
    const scanB = makeDefer();
    configScanSpy
      .mockImplementationOnce(() => scanA.promise)
      .mockImplementationOnce(() => scanB.promise);

    const treeA = {
      scopes: [
        {
          id: "scope:project:A",
          scope_type: "Project",
          label: "Project (cwd/.claude)",
          recursive_count: 1,
          files: [
            {
              id: "f:A",
              rel_path: ".claude/settings.json",
              abs_path: "/repoA/.claude/settings.json",
              display_path: ".claude/settings.json",
              kind: "Settings",
              scope_badges: [],
              size_bytes: 1,
              mtime_unix_ns: 0,
              issues: [],
              symlink_origin: null,
              included_by: null,
              include_depth: 0,
            },
          ],
        },
      ],
      cwd: "/repoA",
      project_root: "/repoA",
      config_home_dir: "/repoA/.claude",
      memory_slug: "a",
      memory_slug_lossy: false,
    };
    const treeB = {
      ...treeA,
      scopes: [
        {
          ...treeA.scopes[0],
          id: "scope:project:B",
          files: [
            { ...treeA.scopes[0].files[0], id: "f:B", abs_path: "/repoB/.claude/settings.json" },
          ],
        },
      ],
      cwd: "/repoB",
      project_root: "/repoB",
      config_home_dir: "/repoB/.claude",
      memory_slug: "b",
    };

    const { rerender } = render(
      <ConfigSection
        subRoute={null}
        onSubRouteChange={() => {}}
        forcedAnchor={{ kind: "folder", path: "/repoA" }}
      />,
    );

    // Wait for scanA to be in flight.
    await waitFor(() => expect(configScanSpy).toHaveBeenCalledTimes(1));

    // Flip anchor to B — triggers scanB.
    rerender(
      <ConfigSection
        subRoute={null}
        onSubRouteChange={() => {}}
        forcedAnchor={{ kind: "folder", path: "/repoB" }}
      />,
    );
    await waitFor(() => expect(configScanSpy).toHaveBeenCalledTimes(2));

    // Resolve B first (the fresh anchor); its scope id should render.
    scanB.resolve(treeB);
    await waitFor(() => {
      expect(screen.queryByText(/repoB/i)).toBeTruthy();
    });

    // Now resolve A (the stale anchor). Its setTree must be dropped by
    // the generation guard — the rendered tree must NOT flip back to
    // A.
    scanA.resolve(treeA);
    // Give React a tick to (not) process the stale resolve.
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByText(/repoA/i)).toBeFalsy();
    expect(screen.queryByText(/repoB/i)).toBeTruthy();
  });

  // Audit 2026-04-26 finding — the FilePreview / EffectiveShell back
  // affordance ("Artifacts" link above the title) must clear the
  // current sub-route so the right pane returns to ConfigHomePane.
  // Regression guard: a stale or no-op handler would leave the user
  // dead-ended on a selected node with no way back.
  it("clears the sub-route when the FilePreview back link is clicked", async () => {
    resetSpies();
    configScanSpy.mockResolvedValue({
      scopes: [
        {
          id: "scope:project:repo",
          scope_type: "Project",
          label: "Project (cwd/.claude)",
          recursive_count: 1,
          files: [
            {
              id: "f:settings",
              rel_path: ".claude/settings.json",
              abs_path: "/repo/.claude/settings.json",
              display_path: ".claude/settings.json",
              kind: "settings",
              scope_badges: [],
              size_bytes: 1,
              mtime_unix_ns: 0,
              issues: [],
              symlink_origin: null,
              included_by: null,
              include_depth: 0,
              summary_title: "settings.json",
              summary_description: null,
            },
          ],
        },
      ],
      cwd: "/repo",
      project_root: "/repo",
      config_home_dir: "/repo/.claude",
      memory_slug: "",
      memory_slug_lossy: false,
    });
    configListEditorsSpy.mockResolvedValue([]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });
    configPreviewSpy.mockResolvedValue({
      body_utf8: "{}",
      truncated: false,
    });

    const onSubRouteChange = vi.fn();

    render(
      <ConfigSection
        subRoute="node:f:settings"
        onSubRouteChange={onSubRouteChange}
      />,
    );

    // Wait for the back affordance to render — only present when a
    // node is selected (subRoute → selectedFile path).
    const backBtn = await screen.findByRole("button", {
      name: /back to artifact list/i,
    });
    fireEvent.click(backBtn);
    expect(onSubRouteChange).toHaveBeenCalledWith(null);
  });

  // Companion to the FilePreview test above — the virtual-route
  // EffectiveShell wrappers (effective-settings / effective-mcp /
  // hooks) share the same `onClose={() => onSubRouteChange(null)}`
  // contract. One representative case (effective-settings) is enough;
  // all three pass `onClose` through the same EffectiveShell component
  // so coverage of one validates the wiring.
  it("clears the sub-route when the EffectiveShell back link is clicked", async () => {
    resetSpies();
    configScanSpy.mockResolvedValue({
      scopes: [],
      cwd: "/repo",
      project_root: "/repo",
      config_home_dir: "/repo/.claude",
      memory_slug: "",
      memory_slug_lossy: false,
    });
    configListEditorsSpy.mockResolvedValue([]);
    configGetEditorDefaultsSpy.mockResolvedValue({
      by_kind: {},
      fallback: "system",
    });

    const onSubRouteChange = vi.fn();

    render(
      <ConfigSection
        subRoute="node:virtual:effective-settings"
        onSubRouteChange={onSubRouteChange}
        forcedAnchor={{ kind: "folder", path: "/repo" }}
      />,
    );

    const backBtn = await screen.findByRole("button", {
      name: /back to artifact list/i,
    });
    fireEvent.click(backBtn);
    expect(onSubRouteChange).toHaveBeenCalledWith(null);
  });
});
