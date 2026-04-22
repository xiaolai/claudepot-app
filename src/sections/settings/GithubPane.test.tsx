/**
 * Verifies the Settings → GitHub pane:
 *   - only the token's last 4 chars ever appear in the DOM
 *   - raw token never leaks through React state after save
 *   - clear calls settingsGithubTokenClear()
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const getSpy = vi.fn();
const setSpy = vi.fn();
const clearSpy = vi.fn();

vi.mock("../../api", () => ({
  api: {
    settingsGithubTokenGet: (...a: unknown[]) => getSpy(...a),
    settingsGithubTokenSet: (...a: unknown[]) => setSpy(...a),
    settingsGithubTokenClear: (...a: unknown[]) => clearSpy(...a),
    // Other sections share this mock module; stub the surface they
    // touch so their mounts (if any) don't blow up.
    preferencesGet: vi.fn().mockResolvedValue({}),
    preferencesSetActivity: vi.fn(),
    preferencesSetNotifications: vi.fn(),
    preferencesSetHideDockIcon: vi.fn(),
    projectList: vi.fn().mockResolvedValue([]),
    protectedPathsList: vi.fn().mockResolvedValue([]),
    repairPendingCount: vi.fn().mockResolvedValue(0),
    activityTrends: vi.fn().mockResolvedValue({}),
    runningOpsList: vi.fn().mockResolvedValue([]),
    appStatus: vi.fn().mockResolvedValue({ data_dir: "/tmp", cc_config_dir: "/tmp" }),
    repairList: vi.fn().mockResolvedValue([]),
    sessionIndexRebuild: vi.fn(),
    projectCleanPreview: vi.fn().mockResolvedValue({ candidates: [] }),
  },
}));

beforeEach(() => {
  getSpy.mockReset();
  setSpy.mockReset();
  clearSpy.mockReset();
});

/**
 * Mount only the GithubPane via a direct import so the whole
 * SettingsSection surface doesn't have to boot. The pane is
 * defined as an inner function, so we re-declare it here for the
 * test — matching the same signature. If the real component's
 * behavior changes, this harness also needs an update.
 */
import { SettingsSection } from "../SettingsSection";

async function mountAndOpenGithubTab() {
  getSpy.mockResolvedValue({ present: false, last4: null, env_override: false });
  render(<SettingsSection />);
  // Click the "GitHub" nav button.
  const nav = await screen.findByRole("button", { name: /GitHub/i });
  await userEvent.click(nav);
}

describe("Settings → GitHub", () => {
  it("shows 'No token stored' when the backend says absent", async () => {
    await mountAndOpenGithubTab();
    await waitFor(() =>
      expect(screen.getByText(/No token stored/)).toBeInTheDocument(),
    );
  });

  it("calls settingsGithubTokenSet with the input and clears the field", async () => {
    getSpy
      .mockResolvedValueOnce({ present: false, last4: null, env_override: false })
      .mockResolvedValueOnce({ present: true, last4: "1234", env_override: false });
    setSpy.mockResolvedValue({ present: true, last4: "1234", env_override: false });
    await mountAndOpenGithubTab();
    const input = await screen.findByLabelText("GitHub token");
    await userEvent.type(input, "ghp_abcdefghij1234");
    await userEvent.click(screen.getByRole("button", { name: /Save/ }));
    await waitFor(() => {
      expect(setSpy).toHaveBeenCalledWith("ghp_abcdefghij1234");
    });
    // Last4 shown after refresh.
    await waitFor(() =>
      expect(screen.getByTestId("github-token-last4")).toHaveTextContent(
        "…1234",
      ),
    );
    // Input cleared — raw token is not retained.
    expect((input as HTMLInputElement).value).toBe("");
    // Raw token is not present anywhere in the rendered DOM.
    expect(document.body.textContent ?? "").not.toContain("ghp_abcdefghij1234");
  });

  it("surfaces a warning when GITHUB_TOKEN env var is set", async () => {
    // `mountAndOpenGithubTab` primes the first call; we want the
    // *second* call (fired when the nav is clicked) to be the env-set
    // one — but in practice the pane only fetches once on mount.
    // So override the default mock before mounting.
    getSpy.mockReset();
    getSpy.mockResolvedValue({
      present: true,
      last4: "1234",
      env_override: true,
    });
    render(<SettingsSection />);
    const nav = await screen.findByRole("button", { name: /GitHub/i });
    await userEvent.click(nav);
    await waitFor(() =>
      expect(
        screen.getByTestId("github-env-override-note"),
      ).toBeInTheDocument(),
    );
  });

  it("Clear calls settingsGithubTokenClear and refreshes", async () => {
    getSpy
      .mockResolvedValueOnce({ present: true, last4: "9999", env_override: false })
      .mockResolvedValueOnce({ present: false, last4: null, env_override: false });
    clearSpy.mockResolvedValue(undefined);
    await mountAndOpenGithubTab();
    await screen.findByTestId("github-token-last4");
    await userEvent.click(screen.getByRole("button", { name: /Clear/ }));
    await waitFor(() => expect(clearSpy).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByText(/No token stored/)).toBeInTheDocument(),
    );
  });
});
