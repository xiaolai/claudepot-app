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
    // GeneralPane mounts the AutoMemoryGlobalRow on first render even
    // when the active tab is "github" — the row's load runs once at
    // mount time. Stub a default shape so the row renders without
    // toasting an error.
    autoMemoryState: vi.fn().mockResolvedValue({
      project_root: "~",
      effective: true,
      decided_by: "default",
      decided_label: "default",
      user_writable: true,
      user_settings_value: null,
      project_settings_value: null,
      local_project_settings_value: null,
      env_disable_set: false,
      env_simple_set: false,
      local_settings_gitignored: null,
    }),
    autoMemoryStateGlobal: vi.fn().mockResolvedValue({
      project_root: "",
      effective: true,
      decided_by: "default",
      decided_label: "default",
      user_writable: true,
      user_settings_value: null,
      project_settings_value: null,
      local_project_settings_value: null,
      env_disable_set: false,
      env_simple_set: false,
      local_settings_gitignored: null,
    }),
    autoMemorySet: vi.fn(),
  },
}));

// SettingsSection now reads pushToast from AppStateProvider; mock the
// provider so the test can mount the section without booting the
// whole app shell. The spy is hoisted so individual tests can assert
// success / error labels and confirm secret redaction wording.
const pushToastSpy = vi.fn();
vi.mock("../../providers/AppStateProvider", () => ({
  useAppState: () => ({ pushToast: pushToastSpy }),
}));

beforeEach(() => {
  getSpy.mockReset();
  setSpy.mockReset();
  clearSpy.mockReset();
  pushToastSpy.mockReset();
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
    // Success toast fires with the saved-not-stringified message —
    // and the raw token never reaches pushToast under any path.
    await waitFor(() =>
      expect(pushToastSpy).toHaveBeenCalledWith("info", "GitHub token saved."),
    );
    for (const call of pushToastSpy.mock.calls) {
      expect(call.join(" ")).not.toContain("ghp_abcdefghij1234");
    }
  });

  it("redacts sk-ant-* tokens that leak into the save-error toast", async () => {
    // Regression guard for the audit-fix follow-up: the catch path now
    // routes through `toastError` so `redactSecrets` scrubs any
    // `sk-ant-*` blob the backend might echo back. We assert two
    // things at once — the scoped label (so the user knows which
    // action failed) and the actual redaction of the token shape the
    // architecture rule names. (GitHub PAT redaction is out of scope
    // for the redactor as currently shipped — see redactSecrets.ts.)
    getSpy.mockResolvedValueOnce({
      present: false,
      last4: null,
      env_override: false,
    });
    const leakedAnthropicToken =
      "sk-ant-oat01-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789xyz";
    setSpy.mockRejectedValueOnce(
      new Error(`backend rejected ${leakedAnthropicToken}`),
    );
    await mountAndOpenGithubTab();
    const input = await screen.findByLabelText("GitHub token");
    await userEvent.type(input, "ghp_abcdefghij1234");
    await userEvent.click(screen.getByRole("button", { name: /Save/ }));

    await waitFor(() => {
      const errorCall = pushToastSpy.mock.calls.find(
        ([kind]) => kind === "error",
      );
      expect(errorCall).toBeDefined();
      const [, message] = errorCall!;
      expect(message).toMatch(/^GitHub token save failed:/);
      // The raw `sk-ant-*` token must not appear in the toast text;
      // the redactor leaves the prefix + masked suffix only.
      expect(message).not.toContain(leakedAnthropicToken);
      expect(message).toMatch(/sk-ant-\*\*\*/);
    });
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
      expect(pushToastSpy).toHaveBeenCalledWith(
        "info",
        "GitHub token cleared.",
      ),
    );
    await waitFor(() =>
      expect(screen.getByText(/No token stored/)).toBeInTheDocument(),
    );
  });
});
