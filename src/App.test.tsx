// React component tests for App.tsx.
//
// Each test mocks @tauri-apps/api/core.invoke with per-command handlers via
// vi.doMock + dynamic import, so App's useEffect/api calls receive controlled
// fixtures. vi.doMock is required (not vi.mock hoisted) so we can configure
// the mock per test.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { sampleAccount, sampleStatus } from "./test/fixtures";

// Dynamic import so each test's mock takes effect before App loads its own
// import of @tauri-apps/api/core.
async function renderApp(handlers: Record<string, (args?: unknown) => unknown>) {
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(async (cmd: string, args?: unknown) => {
      const h = handlers[cmd];
      if (!h) throw new Error(`unmocked Tauri command: ${cmd}`);
      return await h(args);
    }),
  }));
  const { default: App } = await import("./App");
  return render(<App />);
}

beforeEach(() => {
  vi.resetModules();
});

describe("App — initial load", () => {
  it("shows the empty state when no accounts exist", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    expect(await screen.findByText(/No accounts yet/i)).toBeInTheDocument();
    // The add-account buttons — header footer + empty state.
    const addButtons = await screen.findAllByRole("button", {
      name: /add account/i,
    });
    expect(addButtons.length).toBeGreaterThan(0);
  });

  it("renders the account list when the DB is populated", async () => {
    await renderApp({
      app_status: () =>
        sampleStatus({
          account_count: 1,
          cli_active_email: "alice@example.com",
        }),
      account_list: () => [
        sampleAccount({ is_cli_active: true }),
        sampleAccount({
          uuid: "bbbb2222-3333-4444-8555-666666666666",
          email: "bob@example.com",
        }),
      ],
    });

    const aliceEls = await screen.findAllByText("alice@example.com");
    // Alice appears in both the active-CLI pill and her own account card.
    expect(aliceEls.length).toBeGreaterThanOrEqual(2);
    expect(await screen.findByText("bob@example.com")).toBeInTheDocument();
  });

  it("shows a Retry panel when first load fails", async () => {
    const failing = () => {
      throw new Error("connection refused");
    };
    await renderApp({
      app_status: failing,
      account_list: failing,
    });

    expect(
      await screen.findByText(/Couldn't load Claudepot/i),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: /retry/i }),
    ).toBeInTheDocument();
  });
});

describe("AccountCard — button disable logic", () => {
  it("disables Use CLI on the already-active account", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
    });

    const btn = await screen.findByRole("button", { name: /CLI/i });
    expect(btn).toBeDisabled();
    expect(btn).toHaveTextContent(/✓ CLI/);
  });

  it("Log in calls account_login (browser flow) and refreshes", async () => {
    const user = userEvent.setup();
    const login = vi.fn();
    let listCalls = 0;
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => {
        listCalls += 1;
        return listCalls === 1
          ? [
              sampleAccount({
                credentials_healthy: false,
                token_status: "missing",
              }),
            ]
          : [sampleAccount({ credentials_healthy: true })];
      },
      account_login: login,
    });

    await user.click(await screen.findByRole("button", { name: /log in/i }));

    await waitFor(() => {
      expect(login).toHaveBeenCalledWith({
        uuid: "aaaa1111-2222-4333-8444-555555555555",
      });
    });
    // After success the list re-fetches and the button is now Use CLI.
    expect(
      await screen.findByRole("button", { name: /use cli/i }),
    ).toBeInTheDocument();
  });

  it("shows Cancel-login during an in-flight login, calls account_login_cancel", async () => {
    const user = userEvent.setup();
    // account_login resolves only when cancel fires, so we can drive the
    // in-flight state. Use a deferred-like pattern.
    let rejectLogin: ((e: Error) => void) | null = null;
    const cancel = vi.fn(() => {
      rejectLogin?.(new Error("login cancelled"));
    });
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({
          credentials_healthy: false,
          token_status: "missing",
        }),
      ],
      account_login: () =>
        new Promise((_resolve, reject) => {
          rejectLogin = reject;
        }),
      account_login_cancel: cancel,
    });

    await user.click(await screen.findByRole("button", { name: /^log in$/i }));
    // Button flips to Cancel login while the subprocess is running.
    const cancelBtn = await screen.findByRole("button", {
      name: /cancel login/i,
    });
    await user.click(cancelBtn);

    await waitFor(() => {
      expect(cancel).toHaveBeenCalledTimes(1);
    });
    // After the promise rejects with "cancelled", Log in returns.
    expect(
      await screen.findByRole("button", { name: /^log in$/i }),
    ).toBeInTheDocument();
  });

  it("shows Log in (not Use CLI) when the stored blob is unhealthy", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({
          has_cli_credentials: true, // DB says yes, but...
          credentials_healthy: false, // storage says no (stale DB flag)
          token_status: "missing",
        }),
      ],
    });

    // Use CLI disappears; Log in takes its place.
    expect(screen.queryByRole("button", { name: /use cli/i })).toBeNull();
    const btn = await screen.findByRole("button", { name: /log in/i });
    expect(btn).toBeEnabled();
    expect(btn.getAttribute("title")).toMatch(/sign in as/i);
  });

  it("disables Use Desktop when the account has no desktop profile (audit fix #4)", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: false })],
    });

    const btn = await screen.findByRole("button", { name: /use desktop/i });
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toMatch(/no desktop profile yet/i);
  });

  it("disables Use Desktop when Desktop is not installed", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: false }),
      account_list: () => [sampleAccount({ has_desktop_profile: true })],
    });

    const btn = await screen.findByRole("button", { name: /use desktop/i });
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toMatch(/desktop not installed/i);
  });
});

describe("Active pills", () => {
  it("shows em-dash when nothing is active", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    const cliLabel = await screen.findByText("CLI");
    const pill = cliLabel.closest(".pill")!;
    expect(pill.querySelector(".pill-value")?.textContent).toBe("—");
  });

  it("marks the Desktop pill disabled with a hint when Desktop isn't installed", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: false }),
      account_list: () => [],
    });

    const deskLabel = await screen.findByText("Desktop");
    const pill = deskLabel.closest(".pill");
    expect(pill).toHaveClass("disabled");
    expect(pill).toHaveAttribute("title", "Desktop not installed");
  });
});

describe("Add-account modal", () => {
  it("opens the modal when Add is clicked, closes on Cancel", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    const addButtons = await screen.findAllByRole("button", {
      name: /add account/i,
    });
    await user.click(addButtons[0]);

    expect(
      await screen.findByRole("heading", { name: /add account/i }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /cancel/i }));
    await waitFor(() => {
      expect(
        screen.queryByRole("heading", { name: /add account/i }),
      ).not.toBeInTheDocument();
    });
  });

  it("invokes account_add_from_current on submit, refreshes list", async () => {
    const user = userEvent.setup();
    let listCalls = 0;
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => {
        listCalls += 1;
        return listCalls === 1
          ? []
          : [sampleAccount({ email: "newly-added@example.com" })];
      },
      account_add_from_current: () => ({
        email: "newly-added@example.com",
        org_name: "Org",
        subscription_type: "max",
      }),
    });

    const addButtons = await screen.findAllByRole("button", {
      name: /add account/i,
    });
    await user.click(addButtons[0]);

    await user.click(
      await screen.findByRole("button", { name: /add from current/i }),
    );

    // After success, the modal closes and the list is refreshed.
    expect(
      await screen.findByText("newly-added@example.com"),
    ).toBeInTheDocument();
  });

  it("Remove opens an in-app confirm dialog, not window.confirm", async () => {
    // window.confirm can be invisible/suppressed in Tauri webviews. The app
    // uses a state-driven modal instead. A confirm() call must never leak out.
    const user = userEvent.setup();
    const confirmSpy = vi
      .spyOn(window, "confirm")
      .mockImplementation(() => true);

    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
    });

    await user.click(await screen.findByRole("button", { name: /remove/i }));

    // Dialog is visible with the account email highlighted.
    const dialog = await screen.findByRole("dialog", {
      name: /remove account/i,
    });
    expect(within(dialog).getByText("alice@example.com")).toBeInTheDocument();

    // window.confirm must not have been called — that's the whole bug.
    expect(confirmSpy).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("Remove → Cancel closes the dialog without calling account_remove", async () => {
    const user = userEvent.setup();
    const removeMock = vi.fn();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
      account_remove: removeMock,
    });

    await user.click(await screen.findByRole("button", { name: /remove/i }));
    await screen.findByRole("dialog", { name: /remove account/i });

    await user.click(screen.getByRole("button", { name: /cancel/i }));

    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: /remove account/i }),
      ).not.toBeInTheDocument();
    });
    expect(removeMock).not.toHaveBeenCalled();
  });

  it("Remove → Remove confirms, calls account_remove with the right uuid, refreshes", async () => {
    const user = userEvent.setup();
    let listCalls = 0;
    const removeMock = vi.fn(() => ({
      email: "alice@example.com",
      was_cli_active: false,
      was_desktop_active: false,
      had_desktop_profile: false,
      warnings: [],
    }));
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => {
        listCalls += 1;
        return listCalls === 1 ? [sampleAccount()] : [];
      },
      account_remove: removeMock,
    });

    await user.click(await screen.findByRole("button", { name: /remove/i }));
    // Two buttons labeled "Remove" now — the card's original + the dialog's
    // confirm. The dialog one is inside role=dialog.
    const dialog = await screen.findByRole("dialog", {
      name: /remove account/i,
    });
    await user.click(
      await within(dialog).findByRole("button", { name: /remove/i }),
    );

    await waitFor(() => {
      expect(removeMock).toHaveBeenCalledWith({
        uuid: "aaaa1111-2222-4333-8444-555555555555",
      });
    });
    // After success the list re-fetches and shows the empty state.
    expect(await screen.findByText(/No accounts yet/i)).toBeInTheDocument();
  });

  it("does NOT expose any refresh-token input (audit fix #1)", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await user.click(
      (await screen.findAllByRole("button", { name: /add account/i }))[0],
    );

    await screen.findByRole("heading", { name: /add account/i });
    // No input of any kind (a token field would have to be one).
    expect(document.querySelector(".modal input")).toBeNull();
    // The "From refresh token" mode tab is gone.
    expect(
      screen.queryByRole("button", { name: /from refresh token/i }),
    ).toBeNull();
    // The modal body should not contain a UI for pasting a token.
    const modal = document.querySelector(".modal");
    expect(modal?.textContent ?? "").not.toMatch(/paste|sk-ant-ort01/i);
  });
});
