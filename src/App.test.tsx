// React component tests for App.tsx.
//
// Each test mocks @tauri-apps/api/core.invoke with per-command handlers via
// vi.doMock + dynamic import, so App's useEffect/api calls receive controlled
// fixtures. vi.doMock is required (not vi.mock hoisted) so we can configure
// the mock per test.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { sampleAccount, sampleStatus } from "./test/fixtures";

// Dynamic import so each test's mock takes effect before App loads its own
// import of @tauri-apps/api/core.
async function renderApp(handlers: Record<string, (args?: unknown) => unknown>) {
  // Merge defaults for commands most tests don't care about. `fetch_all_usage`
  // and `verify_all_accounts` fire from useEffect on every mount — without a
  // default they'd throw "unmocked command" in every test. verify_all_accounts
  // returning [] means the sidebar keeps the DB-sourced list unchanged.
  // Cache the LATEST account_list result so the default verify pass-through
  // doesn't re-invoke counter-based handlers (many tests use a per-call
  // counter to sequence "before" vs "after" states). After each account_list
  // call we snapshot the result; the default verify_all_accounts returns
  // the most recent snapshot, preserving whatever the test just set up.
  let latestListSnapshot: unknown = [];
  const wrappedAccountList = handlers.account_list
    ? (args?: unknown) => {
        const out = handlers.account_list!(args);
        latestListSnapshot = out;
        return out;
      }
    : undefined;
  const merged: Record<string, (args?: unknown) => unknown> = {
    sync_from_current_cc: () => "",
    fetch_all_usage: () => ({}),
    verify_all_accounts: () => latestListSnapshot,
    // Default: CC has no blob. Tests that want the truth-strip to
    // render a specific identity override this handler.
    current_cc_identity: () => ({
      email: null,
      verified_at: new Date().toISOString(),
      error: null,
    }),
    ...handlers,
    ...(wrappedAccountList ? { account_list: wrappedAccountList } : {}),
  };
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: vi.fn(async (cmd: string, args?: unknown) => {
      const h = merged[cmd];
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

// Helper: select an account in the sidebar by clicking its list item
async function selectAccount(email: string) {
  const user = userEvent.setup();
  // Find the sidebar item containing this email and click it
  const el = await screen.findByText(email);
  const sidebarItem = el.closest(".sidebar-item") ?? el.closest("[role='option']");
  if (sidebarItem) {
    await user.click(sidebarItem as HTMLElement);
  }
}

describe("App — initial load", () => {
  it("shows the empty state when no accounts exist", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    expect(await screen.findByText(/No accounts yet/i)).toBeInTheDocument();
    const addButtons = await screen.findAllByRole("button", {
      name: /add account/i,
    });
    expect(addButtons.length).toBeGreaterThan(0);
  });

  it("renders accounts in the sidebar when the DB is populated", async () => {
    await renderApp({
      app_status: () =>
        sampleStatus({
          account_count: 2,
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

    expect(await screen.findByText("alice@example.com")).toBeInTheDocument();
    expect(await screen.findByText("bob@example.com")).toBeInTheDocument();
  });

  it("shows a Retry panel when first load fails", async () => {
    const failing = () => {
      throw new Error("connection refused");
    };
    await renderApp({
      sync_from_current_cc: failing,
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

describe("WI-1: Window-focus refresh + refresh button", () => {
  it("refresh button calls app_status + account_list", async () => {
    const user = userEvent.setup();
    let statusCalls = 0;
    await renderApp({
      app_status: () => {
        statusCalls += 1;
        return sampleStatus({ account_count: 0 });
      },
      account_list: () => [],
    });

    await screen.findByText(/No accounts yet/i);
    const initialCalls = statusCalls;
    await user.click(screen.getByRole("button", { name: /refresh/i }));
    await waitFor(() => {
      expect(statusCalls).toBeGreaterThan(initialCalls);
    });
  });

  it("focus event triggers refresh after debounce window", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let statusCalls = 0;
    await renderApp({
      app_status: () => {
        statusCalls += 1;
        return sampleStatus({ account_count: 0 });
      },
      account_list: () => [],
    });

    await screen.findByText(/No accounts yet/i);
    const initialCalls = statusCalls;
    // Advance past the 2s debounce window
    vi.advanceTimersByTime(2100);
    fireEvent.focus(window);
    await waitFor(() => {
      expect(statusCalls).toBeGreaterThan(initialCalls);
    });
    vi.useRealTimers();
  });
});

describe("WI-2: Per-account busy states", () => {
  it("login on account A does not disable Use CLI on account B", async () => {
    const user = userEvent.setup();
    const loginState = { resolve: null as (() => void) | null };
    await renderApp({
      app_status: () => sampleStatus({ account_count: 2 }),
      account_list: () => [
        sampleAccount({
          credentials_healthy: false,
          token_status: "missing",
        }),
        sampleAccount({
          uuid: "bbbb2222-3333-4444-8555-666666666666",
          email: "bob@example.com",
          credentials_healthy: true,
        }),
      ],
      account_login: () =>
        new Promise<void>((resolve) => {
          loginState.resolve = resolve;
        }),
      account_login_cancel: () => {},
    });

    // Select alice (unhealthy) and click Log in
    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /log in/i }));

    // Select bob — his "Use CLI" should still be enabled
    await selectAccount("bob@example.com");
    const useCli = await screen.findByRole("button", { name: /use cli/i });
    expect(useCli).not.toBeDisabled();

    // Cleanup
    if (loginState.resolve) loginState.resolve();
  });
});

describe("ContentPane — button disable logic", () => {
  it("disables Use CLI on the already-active account", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
    });

    await selectAccount("alice@example.com");
    const btn = await screen.findByRole("button", { name: /active cli/i });
    expect(btn).toBeDisabled();
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

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /log in/i }));

    await waitFor(() => {
      expect(login).toHaveBeenCalledWith({
        uuid: "aaaa1111-2222-4333-8444-555555555555",
      });
    });
    expect(
      await screen.findByRole("button", { name: /use cli/i }),
    ).toBeInTheDocument();
  });

  it("shows Cancel-login during an in-flight login, calls account_login_cancel", async () => {
    const user = userEvent.setup();
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

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /log in/i }));
    const cancelBtn = await screen.findByRole("button", {
      name: /cancel login/i,
    });
    await user.click(cancelBtn);

    await waitFor(() => {
      expect(cancel).toHaveBeenCalledTimes(1);
    });
    expect(
      await screen.findByRole("button", { name: /log in/i }),
    ).toBeInTheDocument();
  });

  it("shows Log in (not Use CLI) when the stored blob is unhealthy", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({
          has_cli_credentials: true,
          credentials_healthy: false,
          token_status: "missing",
        }),
      ],
    });

    await selectAccount("alice@example.com");
    expect(screen.queryByRole("button", { name: /use cli/i })).toBeNull();
    const btn = await screen.findByRole("button", { name: /log in/i });
    expect(btn).toBeEnabled();
    expect(btn.getAttribute("title")).toMatch(/sign in as/i);
  });

  it("disables Use Desktop when the account has no desktop profile", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: false })],
    });

    await selectAccount("alice@example.com");
    const btn = await screen.findByRole("button", { name: /use desktop/i });
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toMatch(/no desktop profile/i);
  });

  it("disables Use Desktop when Desktop is not installed", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: false }),
      account_list: () => [sampleAccount({ has_desktop_profile: true })],
    });

    await selectAccount("alice@example.com");
    const btn = await screen.findByRole("button", { name: /use desktop/i });
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toMatch(/desktop not installed/i);
  });
});

describe("WI-4: Escape key closes modals", () => {
  it("pressing Escape on ConfirmDialog calls onCancel", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
    });

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /remove/i }));
    await screen.findByRole("dialog", { name: /remove account/i });

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: /remove account/i }),
      ).not.toBeInTheDocument();
    });
  });

  it("pressing Escape on AddAccountModal closes it", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await user.click(
      (await screen.findAllByRole("button", { name: /add account/i }))[0],
    );
    await screen.findByRole("dialog");

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });
});

describe("WI-5: CLI Clear button", () => {
  it("shows Clear CLI button when cli_active_email is set", async () => {
    await renderApp({
      app_status: () =>
        sampleStatus({ cli_active_email: "alice@example.com" }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
    });

    await selectAccount("alice@example.com");
    expect(
      await screen.findByRole("button", { name: /clear cli/i }),
    ).toBeInTheDocument();
  });

  it("hides Clear CLI button when no CLI account is active", async () => {
    await renderApp({
      app_status: () => sampleStatus({ cli_active_email: null }),
      account_list: () => [sampleAccount()],
    });

    await selectAccount("alice@example.com");
    // Email appears in sidebar + content + detail — use getAllByText
    const aliceEls = await screen.findAllByText("alice@example.com");
    expect(aliceEls.length).toBeGreaterThanOrEqual(1);
    expect(
      screen.queryByRole("button", { name: /clear cli/i }),
    ).not.toBeInTheDocument();
  });

  it("confirm + invoke clears CLI and refreshes", async () => {
    const user = userEvent.setup();
    const clearMock = vi.fn();
    let listCalls = 0;
    await renderApp({
      app_status: () =>
        sampleStatus({ cli_active_email: "alice@example.com" }),
      account_list: () => {
        listCalls += 1;
        return [sampleAccount({ is_cli_active: listCalls === 1 })];
      },
      cli_clear: clearMock,
    });

    await selectAccount("alice@example.com");
    await user.click(
      await screen.findByRole("button", { name: /clear cli/i }),
    );
    // Confirm dialog appears
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /clear/i }));

    await waitFor(() => {
      expect(clearMock).toHaveBeenCalledTimes(1);
    });
  });
});

describe("WI-6: Desktop switch confirmation", () => {
  it("Use Desktop opens confirm dialog", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: true })],
    });

    const user = userEvent.setup();
    await selectAccount("alice@example.com");
    await user.click(
      await screen.findByRole("button", { name: /use desktop/i }),
    );

    expect(
      await screen.findByRole("dialog", { name: /switch desktop/i }),
    ).toBeInTheDocument();
  });

  it("confirm calls desktop_use", async () => {
    const user = userEvent.setup();
    const desktopUse = vi.fn();
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: true })],
      desktop_use: desktopUse,
    });

    await selectAccount("alice@example.com");
    await user.click(
      await screen.findByRole("button", { name: /use desktop/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /switch/i }));

    await waitFor(() => {
      expect(desktopUse).toHaveBeenCalled();
    });
  });

  it("cancel closes dialog without calling desktop_use", async () => {
    const user = userEvent.setup();
    const desktopUse = vi.fn();
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: true })],
      desktop_use: desktopUse,
    });

    await selectAccount("alice@example.com");
    await user.click(
      await screen.findByRole("button", { name: /use desktop/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(desktopUse).not.toHaveBeenCalled();
  });
});

describe("WI-7: Active-slot badges", () => {
  it("CLI-active account shows CLI badge in detail view", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
    });

    await selectAccount("alice@example.com");
    // Detail header has text badge; sidebar uses an icon (aria-label).
    expect(screen.getByText("CLI")).toHaveClass("slot-badge");
    expect(screen.getByLabelText("Active CLI account")).toBeInTheDocument();
  });

  it("both-active account shows both badges", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({ is_cli_active: true, is_desktop_active: true }),
      ],
    });

    await selectAccount("alice@example.com");
    const badges = screen.getAllByText(/^(CLI|Desktop)$/);
    const slotBadges = badges.filter((b) => b.classList.contains("slot-badge"));
    expect(slotBadges).toHaveLength(2); // detail-header text badges
    // Sidebar icons checked by aria-label
    expect(screen.getByLabelText("Active CLI account")).toBeInTheDocument();
    expect(screen.getByLabelText("Active Desktop account")).toBeInTheDocument();
  });

  it("inactive account shows no slot badges", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({ is_cli_active: false, is_desktop_active: false }),
      ],
    });

    await selectAccount("alice@example.com");
    expect(document.querySelectorAll(".slot-badge")).toHaveLength(0);
  });
});

describe("WI-8: Persistent error toasts", () => {
  it("error toast has a dismiss button", async () => {
    const user = userEvent.setup();
    const failing = () => {
      throw new Error("fail");
    };
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
      cli_use: failing,
    });

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /use cli/i }));

    const toast = await screen.findByText(/CLI switch failed/i);
    const closeBtn = toast.closest(".toast")!.querySelector(".toast-close");
    expect(closeBtn).toBeTruthy();
  });

  it("clicking dismiss removes the toast", async () => {
    const user = userEvent.setup();
    const failing = () => {
      throw new Error("fail");
    };
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
      cli_use: failing,
    });

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /use cli/i }));

    const toastText = await screen.findByText(/CLI switch failed/i);
    const closeBtn = toastText.closest(".toast")!.querySelector(".toast-close") as HTMLElement;
    await user.click(closeBtn);

    await waitFor(() => {
      expect(screen.queryByText(/CLI switch failed/i)).not.toBeInTheDocument();
    });
  });
});

describe("WI-9: Inline disabled-button reasons", () => {
  it("has_desktop_profile: false renders hint text", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [sampleAccount({ has_desktop_profile: false })],
    });

    await selectAccount("alice@example.com");
    expect(
      await screen.findByText(/no desktop profile/i),
    ).toBeInTheDocument();
  });

  it("healthy account with no disabled buttons renders no hint", async () => {
    await renderApp({
      app_status: () => sampleStatus({ desktop_installed: true }),
      account_list: () => [
        sampleAccount({ has_desktop_profile: true, credentials_healthy: true }),
      ],
    });

    await selectAccount("alice@example.com");
    const aliceEls = await screen.findAllByText("alice@example.com");
    expect(aliceEls.length).toBeGreaterThanOrEqual(1);
    expect(document.querySelector(".account-hint")).toBeNull();
  });
});

describe("WI-11: Account detail panel", () => {
  it("selecting account shows detail with UUID", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
    });

    await selectAccount("alice@example.com");
    expect(await screen.findByText("aaaa1111-2222-4333-8444-555555555555")).toBeInTheDocument();
  });

  it("detail shows UUID and timestamps", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({
        last_cli_switch: new Date(Date.now() - 3600000).toISOString(),
      })],
    });

    await selectAccount("alice@example.com");
    expect(await screen.findByText("aaaa1111-2222-4333-8444-555555555555")).toBeInTheDocument();
    expect(await screen.findByText(/1h ago/)).toBeInTheDocument();
  });
});

describe("WI-13: AddAccountModal accessibility", () => {
  it("AddAccountModal has role=dialog with unique aria-labelledby", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await user.click(
      (await screen.findAllByRole("button", { name: /add account/i }))[0],
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    const labelledBy = dialog.getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    // Heading id must match aria-labelledby
    const heading = dialog.querySelector("h2");
    expect(heading?.id).toBe(labelledBy);
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

    // Email may appear in multiple places after auto-select fires
    // (sidebar + detail header + detail row). Use findAllByText.
    const matches = await screen.findAllByText("newly-added@example.com");
    expect(matches.length).toBeGreaterThan(0);
  });

  it("Remove opens an in-app confirm dialog, not window.confirm", async () => {
    const user = userEvent.setup();
    const confirmSpy = vi
      .spyOn(window, "confirm")
      .mockImplementation(() => true);

    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
    });

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /remove/i }));

    const dialog = await screen.findByRole("dialog", {
      name: /remove account/i,
    });
    expect(within(dialog).getByText("alice@example.com")).toBeInTheDocument();

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

    await selectAccount("alice@example.com");
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

    await selectAccount("alice@example.com");
    await user.click(await screen.findByRole("button", { name: /remove/i }));
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
  });

  it("does NOT expose any refresh-token input", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await user.click(
      (await screen.findAllByRole("button", { name: /add account/i }))[0],
    );

    await screen.findByRole("heading", { name: /add account/i });
    expect(document.querySelector(".modal input")).toBeNull();
    expect(
      screen.queryByRole("button", { name: /from refresh token/i }),
    ).toBeNull();
    const modal = document.querySelector(".modal");
    expect(modal?.textContent ?? "").not.toMatch(/paste|sk-ant-ort01/i);
  });
});

describe("AccountDetail fields", () => {
  it("renders all metadata fields", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({
        last_cli_switch: new Date(Date.now() - 120000).toISOString(),
        last_desktop_switch: null,
        has_desktop_profile: false,
      })],
    });

    await selectAccount("alice@example.com");

    // UUID
    expect(await screen.findByText("aaaa1111-2222-4333-8444-555555555555")).toBeInTheDocument();
    // Org — appears in sidebar meta and content detail, use getAll
    const orgEls = screen.getAllByText("Alice Org");
    expect(orgEls.length).toBeGreaterThanOrEqual(1);
    // Relative time
    expect(screen.getByText("2m ago")).toBeInTheDocument();
    // Null timestamp
    const dashes = screen.getAllByText("—");
    expect(dashes.length).toBeGreaterThanOrEqual(1);
    // Credential health
    expect(screen.getByText("healthy")).toBeInTheDocument();
    // Desktop profile
    expect(screen.getByText("none")).toBeInTheDocument();
  });
});

describe("WI-3: Error boundary", () => {
  it("renders fallback when a child throws", async () => {
    // Suppress React error boundary console noise
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});

    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: vi.fn(async () => {
        throw new Error("boom");
      }),
    }));

    // Import a component that will throw during render via the hook
    const { ErrorBoundary } = await import("./ErrorBoundary");
    const Bomb = () => { throw new Error("render crash"); };

    render(
      <ErrorBoundary>
        <Bomb />
      </ErrorBoundary>,
    );

    expect(await screen.findByText(/something went wrong/i)).toBeInTheDocument();
    expect(screen.getByText("render crash")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();

    spy.mockRestore();
  });
});

describe("Concurrent refresh guard", () => {
  it("does not fire overlapping refresh calls", async () => {
    const user = userEvent.setup();
    let concurrentCalls = 0;
    let maxConcurrent = 0;
    await renderApp({
      app_status: () => {
        concurrentCalls += 1;
        maxConcurrent = Math.max(maxConcurrent, concurrentCalls);
        return new Promise(resolve => {
          setTimeout(() => {
            concurrentCalls -= 1;
            resolve(sampleStatus({ account_count: 0 }));
          }, 100);
        });
      },
      account_list: () => [],
    });

    await screen.findByText(/No accounts yet/i);

    // Rapid triple-click refresh
    const btn = screen.getByRole("button", { name: /refresh/i });
    await user.click(btn);
    await user.click(btn);
    await user.click(btn);

    await waitFor(() => {
      // Should never have more than 1 concurrent app_status call
      expect(maxConcurrent).toBeLessThanOrEqual(1);
    });
  });
});

describe("App — verified identity surface", () => {
  it("renders the drift banner when any account has drift=true", async () => {
    const driftedAccount = sampleAccount({
      email: "lixiaolai@gmail.com",
      verified_email: "xiaolaiapple@gmail.com",
      verify_status: "drift",
      drift: true,
    });
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [driftedAccount],
    });

    // Banner text comes from App.tsx's drift alert block.
    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent(/account drift detected/i);
    expect(banner).toHaveTextContent(
      /lixiaolai@gmail\.com authenticates as xiaolaiapple@gmail\.com/i,
    );
  });

  it("does not render the drift banner when all accounts have drift=false", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({ drift: false, verify_status: "ok" })],
    });

    await screen.findByText("alice@example.com");
    // Keychain banner uses role="alert" too — assert on the drift text only.
    expect(screen.queryByText(/account drift detected/i)).not.toBeInTheDocument();
  });

  it("sidebar status dot gets a DRIFT tooltip when the slot is misfiled", async () => {
    const drifted = sampleAccount({
      email: "lixiaolai@gmail.com",
      verified_email: "xiaolaiapple@gmail.com",
      verify_status: "drift",
      drift: true,
      token_status: "valid (7h 59m remaining)",
    });
    const { container } = await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [drifted],
    });

    await screen.findByText("lixiaolai@gmail.com");
    // status-dot sits inside .sidebar-item; title attr carries the tooltip.
    const dot = container.querySelector(".sidebar-item .status-dot");
    expect(dot).not.toBeNull();
    expect(dot).toHaveAttribute(
      "title",
      expect.stringContaining("DRIFT — blob authenticates as xiaolaiapple@gmail.com"),
    );
    // Drift must override token_status for color: dot has .bad class
    // regardless of the locally-valid token.
    expect(dot?.className).toContain("bad");
  });
});

describe("App — CC truth strip + sync banner", () => {
  it("renders CC identity in the truth strip when /profile returns an email", async () => {
    await renderApp({
      app_status: () =>
        sampleStatus({
          account_count: 1,
          cli_active_email: "alice@example.com",
        }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
      current_cc_identity: () => ({
        email: "alice@example.com",
        verified_at: new Date().toISOString(),
        error: null,
      }),
    });

    const strip = await screen.findByLabelText(/CC authentication status/i);
    expect(strip).toHaveTextContent(/alice@example\.com/);
    expect(within(strip).getByText(/MATCH/i)).toBeInTheDocument();
  });

  it("truth strip shows DRIFT when CC identity differs from Claudepot's active_cli", async () => {
    await renderApp({
      app_status: () =>
        sampleStatus({
          account_count: 1,
          cli_active_email: "lixiaolai@gmail.com",
        }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
      current_cc_identity: () => ({
        email: "xiaolaiapple@gmail.com",
        verified_at: new Date().toISOString(),
        error: null,
      }),
    });

    const strip = await screen.findByLabelText(/CC authentication status/i);
    expect(strip).toHaveTextContent(/xiaolaiapple@gmail\.com/);
    expect(within(strip).getByText(/DRIFT/i)).toBeInTheDocument();
  });

  it("truth strip surfaces a CC /profile error instead of staying silent", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
      current_cc_identity: () => ({
        email: null,
        verified_at: new Date().toISOString(),
        error: "access token rejected by /api/oauth/profile",
      }),
    });

    const strip = await screen.findByLabelText(/CC authentication status/i);
    expect(strip).toHaveTextContent(
      /could not verify.*access token rejected/i,
    );
  });

  it("shows a sync-failure banner when sync_from_current_cc throws", async () => {
    await renderApp({
      sync_from_current_cc: () => {
        throw new Error("access token rejected by /api/oauth/profile");
      },
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    const alerts = await screen.findAllByRole("alert");
    const syncBanner = alerts.find((el) =>
      /couldn't sync with claude code/i.test(el.textContent ?? ""),
    );
    expect(syncBanner).toBeDefined();
    expect(syncBanner).toHaveTextContent(/access token rejected/);
  });
});

describe("AccountDetail — Verified row", () => {
  it("renders 'verified as X' when verify_status is ok", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({
          verify_status: "ok",
          verified_email: "alice@example.com",
          verified_at: new Date().toISOString(),
        }),
      ],
    });

    await selectAccount("alice@example.com");
    // "Verified" dt label with a dd whose verify-line.ok span contains the email.
    const verifiedDt = await screen.findByText(/^Verified$/i);
    const verifiedDd = verifiedDt.nextElementSibling as HTMLElement;
    expect(verifiedDd).toHaveTextContent(/alice@example\.com/);
    expect(verifiedDd.querySelector(".verify-line.ok")).not.toBeNull();
  });

  it("renders DRIFT text in the Verified row when the slot is misfiled", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({
          email: "lixiaolai@gmail.com",
          verify_status: "drift",
          verified_email: "xiaolaiapple@gmail.com",
          drift: true,
          verified_at: new Date().toISOString(),
        }),
      ],
    });

    await selectAccount("lixiaolai@gmail.com");
    const verifiedDt = await screen.findByText(/^Verified$/i);
    const verifiedDd = verifiedDt.nextElementSibling as HTMLElement;
    expect(verifiedDd).toHaveTextContent(
      /DRIFT — blob authenticates as xiaolaiapple@gmail\.com/,
    );
    expect(verifiedDd.querySelector(".verify-line.bad")).not.toBeNull();
  });

  it("renders 'not past local expiry' qualifier on a valid token", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({ token_status: "valid (7h 59m remaining)" }),
      ],
    });

    await selectAccount("alice@example.com");
    const tokenDt = await screen.findByText(/^Token$/i);
    const tokenDd = tokenDt.nextElementSibling as HTMLElement;
    expect(tokenDd).toHaveTextContent(/not past local expiry/i);
  });
});
