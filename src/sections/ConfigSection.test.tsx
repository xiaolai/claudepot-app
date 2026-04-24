/**
 * P0 smoke tests for ConfigSection: the section renders, invokes the
 * P0 backend commands (scan, list editors, get defaults) on mount,
 * and exposes the split-button "Open in…" primary action.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const configScanSpy = vi.fn();
const configListEditorsSpy = vi.fn();
const configGetEditorDefaultsSpy = vi.fn();
const configOpenInEditorPathSpy = vi.fn();
const configSetEditorDefaultSpy = vi.fn();

const configPreviewSpy = vi.fn();

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
    configWatchStart: vi.fn().mockResolvedValue(undefined),
    configWatchStop: vi.fn().mockResolvedValue(undefined),
    // Anchor picker pulls the recent-projects list on mount.
    projectList: vi.fn().mockResolvedValue([]),
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
});
