import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectPermission } from "../../api/permission";

const getImpl = { fn: vi.fn() };
const grantSpy = vi.fn();
const revertSpy = vi.fn();
const clearSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    permissionGet: (...args: unknown[]) => getImpl.fn(...args),
    permissionGrant: (...args: unknown[]) => grantSpy(...args),
    permissionRevert: (...args: unknown[]) => revertSpy(...args),
    permissionExtend: vi.fn(),
    permissionClearIgnored: (...args: unknown[]) => clearSpy(...args),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));
const pushToast = vi.fn();
vi.mock("../../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast }),
}));

import { PermissionPanel } from "./PermissionPanel";

const PROJECT = "/Users/me/proj";

function perm(over: Partial<ProjectPermission> = {}): ProjectPermission {
  return {
    projectPath: PROJECT,
    effectiveMode: "default",
    decidedBy: "default",
    isElevated: false,
    ignoredValue: null,
    activeGrant: null,
    hookInstalled: false,
    projectScopeIgnoresSince: "2.1.257",
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getImpl.fn = vi.fn(() => Promise.resolve(perm()));
});

describe("PermissionPanel", () => {
  it("grants through the hook with the selected duration and shows the countdown", async () => {
    const granted = perm({
      activeGrant: { grantedAtMs: Date.now(), expiresAtMs: Date.now() + 30 * 60_000 },
      hookInstalled: true,
    });
    grantSpy.mockResolvedValue(granted);
    render(<PermissionPanel projectPath={PROJECT} />);

    const button = await screen.findByRole("button", { name: /grant auto-approval/i });
    // The intro says what a grant is and what it is not.
    expect(screen.getByText(/PreToolUse/)).toBeInTheDocument();
    await userEvent.click(button);

    // The first preset is 30 minutes; no `mode` argument any more —
    // a grant is not a settings value.
    expect(grantSpy).toHaveBeenCalledWith(PROJECT, 30 * 60);
    expect(await screen.findByRole("status")).toHaveTextContent(/auto-approval active/i);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says so when the grant has no hook behind it", async () => {
    // The one state the pane must never render as plainly "active":
    // a grant CC will never be asked about.
    getImpl.fn = vi.fn(() =>
      Promise.resolve(
        perm({
          activeGrant: { grantedAtMs: Date.now(), expiresAtMs: null },
          hookInstalled: false,
        }),
      ),
    );
    render(<PermissionPanel projectPath={PROJECT} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/hook .* not installed/i);
    expect(screen.getByRole("status")).toHaveTextContent(/stays until you revoke/i);
  });

  it("renders a stale local bypassPermissions as ignored, not elevated, and can remove it", async () => {
    getImpl.fn = vi.fn(() =>
      Promise.resolve(
        perm({
          decidedBy: "project_scope_ignored",
          ignoredValue: { layer: "local_project", mode: "bypassPermissions" },
        }),
      ),
    );
    clearSpy.mockResolvedValue(perm());
    render(<PermissionPanel projectPath={PROJECT} />);

    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent(/2\.1\.257/);
    expect(note).toHaveTextContent(/settings\.local\.json/);
    expect(screen.queryByText(/^elevated$/)).not.toBeInTheDocument();
    expect(screen.getByText(/^ignored$/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /remove it/i }));
    expect(clearSpy).toHaveBeenCalledWith(PROJECT);
    await waitFor(() => expect(screen.queryByRole("note")).not.toBeInTheDocument());
    // The grant form is still offered underneath — an ignored key is
    // not a reason to withhold a real grant.
    expect(screen.getByRole("button", { name: /grant auto-approval/i })).toBeInTheDocument();
  });

  it("offers no removal for a value in the committed project file", async () => {
    getImpl.fn = vi.fn(() =>
      Promise.resolve(
        perm({
          decidedBy: "project_scope_ignored",
          ignoredValue: { layer: "project", mode: "auto" },
        }),
      ),
    );
    render(<PermissionPanel projectPath={PROJECT} />);
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent(/by hand/i);
    expect(screen.queryByRole("button", { name: /remove it/i })).not.toBeInTheDocument();
  });

  it("shows a user-settings bypass as elevated by hand, with nothing to revoke", async () => {
    getImpl.fn = vi.fn(() =>
      Promise.resolve(
        perm({
          effectiveMode: "bypassPermissions",
          decidedBy: "user_settings",
          isElevated: true,
        }),
      ),
    );
    render(<PermissionPanel projectPath={PROJECT} />);
    expect(await screen.findByText(/^elevated$/)).toBeInTheDocument();
    expect(screen.getByText(/not a Claudepot grant/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /revoke now/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /grant auto-approval/i })).not.toBeInTheDocument();
  });

  it("revokes a grant", async () => {
    getImpl.fn = vi.fn(() =>
      Promise.resolve(
        perm({
          activeGrant: { grantedAtMs: Date.now(), expiresAtMs: null },
          hookInstalled: true,
        }),
      ),
    );
    revertSpy.mockResolvedValue(perm());
    render(<PermissionPanel projectPath={PROJECT} />);
    await userEvent.click(await screen.findByRole("button", { name: /revoke now/i }));
    expect(revertSpy).toHaveBeenCalledWith(PROJECT);
    expect(await screen.findByRole("button", { name: /grant auto-approval/i })).toBeInTheDocument();
    expect(pushToast).toHaveBeenCalledWith("info", expect.stringMatching(/revoked/i));
  });
});
