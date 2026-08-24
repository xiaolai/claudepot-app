/**
 * #89: a channel Claudepot cannot track must be named, not mislabelled.
 *
 * CC 2.1.241's schema is `["latest","stable","rc"]` and there is no
 * published feed for `rc` (`GET /claude-code-releases/rc` → 404). The
 * backend used to coerce any unrecognized value to `latest`, so a user
 * on `rc` saw the `latest` button lit and a version comparison against
 * the `latest` baseline. Both are wrong, and neither throws.
 *
 * The backend half is pinned by `CcChannel`'s tests in
 * `updates::version`; this is the half a user actually looks at.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const updatesStatusGetSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    updatesStatusGet: (...a: unknown[]) => updatesStatusGetSpy(...a),
    updatesCheckNow: vi.fn(),
    updatesChannelSet: vi.fn(),
    updatesCliInstall: vi.fn(),
    updatesDesktopInstall: vi.fn(),
    updatesSettingsSet: vi.fn(),
    updatesMinimumVersionSet: vi.fn(),
  },
}));

import { UpdatesPanel } from "./UpdatesPanel";
import type { UpdatesStatusDto } from "../../types/updates";

const status = (channel: string, last_known: string | null): UpdatesStatusDto => ({
  cli: {
    channel,
    installs: [],
    latest_remote: null,
    last_known,
    last_check_unix: null,
    last_error: null,
    cc_settings: {
      auto_updates_channel: channel,
      minimum_version: null,
      disable_autoupdater: false,
      disable_updates: false,
    },
    running_count: 0,
  },
  desktop: {
    install: null,
    running: false,
    latest_remote: null,
    latest_commit_sha: null,
    last_check_unix: null,
    last_error: null,
  },
  settings: {
    cli: {
      notify_on_available: false,
      notify_os_on_available: false,
      force_update_on_check: false,
    },
    desktop: {
      notify_on_available: false,
      notify_os_on_available: false,
      auto_install_when_quit: false,
    },
    poll_interval_minutes: null,
  },
  cli_auto_outcome: { kind: "disabled" },
  desktop_auto_outcome: { kind: "disabled" },
});

beforeEach(() => updatesStatusGetSpy.mockReset());

describe("UpdatesPanel channel row", () => {
  it("names an untracked channel instead of lighting a button for it", async () => {
    updatesStatusGetSpy.mockResolvedValue(status("rc", null));
    render(<UpdatesPanel />);
    await waitFor(() => expect(updatesStatusGetSpy).toHaveBeenCalled());

    // The value CC actually holds is on screen.
    expect(screen.getByText("rc")).toBeInTheDocument();

    // And neither switchable channel claims to be the active one —
    // this is the assertion that fails against the old coercion.
    for (const name of ["latest", "stable"]) {
      const btn = screen.getByRole("button", { name });
      expect(btn.getAttribute("aria-pressed")).not.toBe("true");
    }
  });

  it("still marks the active channel when it is one we track", async () => {
    updatesStatusGetSpy.mockResolvedValue(status("stable", "2.1.231"));
    render(<UpdatesPanel />);
    await waitFor(() => expect(updatesStatusGetSpy).toHaveBeenCalled());

    expect(
      screen.getByRole("button", { name: "stable" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: "latest" }).getAttribute("aria-pressed"),
    ).not.toBe("true");
  });
});
