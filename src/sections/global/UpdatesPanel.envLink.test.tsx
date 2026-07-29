/**
 * D3: Env Variables owns `DISABLE_AUTOUPDATER` / `DISABLE_UPDATES`, and
 * Updates links to it.
 *
 * There has never been a writer for those two keys anywhere — Updates
 * only reports them as blockers — so a user told "manual updates are
 * blocked" had no way to act on it. Duplicating a control here would
 * manufacture the ownership conflict rather than resolve it, so the
 * affordance is a deep link and nothing more.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

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

const status = (
  disable_autoupdater: boolean,
  disable_updates: boolean,
): UpdatesStatusDto => ({
  cli: {
    channel: "latest",
    installs: [],
    latest_remote: null,
    last_known: null,
    last_check_unix: null,
    last_error: null,
    cc_settings: {
      auto_updates_channel: null,
      minimum_version: null,
      disable_autoupdater,
      disable_updates,
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

describe("UpdatesPanel → Env Variables deep link", () => {
  it("offers no link when neither flag is set", async () => {
    updatesStatusGetSpy.mockResolvedValue(status(false, false));
    render(<UpdatesPanel onEditEnvVars={() => {}} />);
    await waitFor(() => expect(updatesStatusGetSpy).toHaveBeenCalled());
    expect(
      screen.queryByRole("button", { name: "Edit in Env variables" }),
    ).toBeNull();
  });

  it("turns each blocker into something the user can act on", async () => {
    const onEditEnvVars = vi.fn();
    updatesStatusGetSpy.mockResolvedValue(status(true, true));
    render(<UpdatesPanel onEditEnvVars={onEditEnvVars} />);

    const links = await screen.findAllByRole("button", {
      name: "Edit in Env variables",
    });
    // One per blocker, so whichever warning the user is reading is
    // actionable from where they are reading it.
    expect(links).toHaveLength(2);

    await userEvent.click(links[0]);
    expect(onEditEnvVars).toHaveBeenCalledTimes(1);
  });

  it("still renders standalone when no handler is supplied", async () => {
    updatesStatusGetSpy.mockResolvedValue(status(false, true));
    render(<UpdatesPanel />);
    const link = await screen.findByRole("button", {
      name: "Edit in Env variables",
    });
    await userEvent.click(link);
    expect(link).toBeInTheDocument();
  });
});
