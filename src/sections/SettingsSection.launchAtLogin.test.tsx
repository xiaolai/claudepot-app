/**
 * Launch-at-login is release-only.
 *
 * `tauri-plugin-autostart` registers `current_exe()` verbatim, so a dev
 * build that flipped the toggle installed `target/debug/claudepot-tauri`
 * as the login item, and every login after that opened a blank window
 * (2026-08-26). `lib.rs` no longer registers the plugin in debug builds;
 * this pins the visible half of that contract — the row exists exactly
 * when `AppStatus.dev_build` is false, and a dev build never even asks
 * the plugin.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { sampleStatus } from "../test/fixtures";

const appStatusSpy = vi.fn();
const isEnabledSpy = vi.fn();

vi.mock("../api", () => ({
  api: {
    appStatus: (...a: unknown[]) => appStatusSpy(...a),
    preferencesGet: vi.fn().mockResolvedValue({
      hide_dock_icon: false,
      show_window_on_startup: false,
      locale: null,
    }),
  },
}));

vi.mock("@tauri-apps/plugin-autostart", () => ({
  isEnabled: (...a: unknown[]) => isEnabledSpy(...a),
  enable: vi.fn(),
  disable: vi.fn(),
}));

import { GeneralPane } from "./SettingsSection";

const LAUNCH_AT_LOGIN = { name: "Launch at login" };

describe("GeneralPane — launch at login is release-only", () => {
  beforeEach(() => {
    appStatusSpy.mockReset();
    isEnabledSpy.mockReset();
    isEnabledSpy.mockResolvedValue(true);
  });

  it("release build: offers the toggle and reads its state from the plugin", async () => {
    appStatusSpy.mockResolvedValue(sampleStatus({ dev_build: false }));
    render(<GeneralPane pushToast={vi.fn()} />);
    expect(
      await screen.findByRole("switch", LAUNCH_AT_LOGIN),
    ).toBeInTheDocument();
    expect(isEnabledSpy).toHaveBeenCalledTimes(1);
  });

  it("dev build: hides the toggle and never asks the plugin", async () => {
    appStatusSpy.mockResolvedValue(sampleStatus({ dev_build: true }));
    render(<GeneralPane pushToast={vi.fn()} />);
    // "Status loaded" has to be observed through something that only
    // renders after it: the macOS-only rows (`sampleStatus` is macos)
    // appear once `isMac` flips, growing the switch count. Waiting on
    // the absence of our row alone would pass before the effect ran.
    const before = screen.getAllByRole("switch").length;
    await waitFor(() =>
      expect(screen.getAllByRole("switch").length).toBeGreaterThan(before),
    );
    expect(screen.queryByRole("switch", LAUNCH_AT_LOGIN)).toBeNull();
    expect(isEnabledSpy).not.toHaveBeenCalled();
  });
});
