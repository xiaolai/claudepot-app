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
  // Merge in sync_from_current_cc default (many tests don't care about it)
  const merged = { sync_from_current_cc: () => "", ...handlers };
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
    expect(aliceEls.length).toBeGreaterThanOrEqual(2);
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
    let resolveLogin: (() => void) | null = null;
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
          resolveLogin = resolve;
        }),
      account_login_cancel: () => {},
    });

    // Click Log in on alice (unhealthy account)
    await user.click(await screen.findByRole("button", { name: /^log in$/i }));

    // Bob's "Use CLI" should still be enabled
    const bobCard = (await screen.findByText("bob@example.com")).closest("article")!;
    const useCli = within(bobCard).getByRole("button", { name: /use cli/i });
    expect(useCli).not.toBeDisabled();

    // Cleanup
    resolveLogin?.();
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

    await user.click(await screen.findByRole("button", { name: /^log in$/i }));
    const cancelBtn = await screen.findByRole("button", {
      name: /cancel login/i,
    });
    await user.click(cancelBtn);

    await waitFor(() => {
      expect(cancel).toHaveBeenCalledTimes(1);
    });
    expect(
      await screen.findByRole("button", { name: /^log in$/i }),
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

describe("WI-4: Escape key closes modals", () => {
  it("pressing Escape on ConfirmDialog calls onCancel", async () => {
    const user = userEvent.setup();
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount()],
    });

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

    expect(
      await screen.findByRole("button", { name: /clear cli/i }),
    ).toBeInTheDocument();
  });

  it("hides Clear CLI button when no CLI account is active", async () => {
    await renderApp({
      app_status: () => sampleStatus({ cli_active_email: null }),
      account_list: () => [sampleAccount()],
    });

    await screen.findByText("alice@example.com");
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
  it("CLI-active account shows CLI badge", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [sampleAccount({ is_cli_active: true })],
    });

    const card = (await screen.findByText("alice@example.com")).closest("article")!;
    expect(within(card).getByText("CLI")).toHaveClass("slot-badge");
  });

  it("both-active account shows both badges", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({ is_cli_active: true, is_desktop_active: true }),
      ],
    });

    const card = (await screen.findByText("alice@example.com")).closest("article")!;
    const badges = within(card).getAllByText(/^(CLI|Desktop)$/);
    const slotBadges = badges.filter((b) => b.classList.contains("slot-badge"));
    expect(slotBadges).toHaveLength(2);
  });

  it("inactive account shows no slot badges", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 1 }),
      account_list: () => [
        sampleAccount({ is_cli_active: false, is_desktop_active: false }),
      ],
    });

    await screen.findByText("alice@example.com");
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

    await screen.findByText("alice@example.com");
    expect(document.querySelector(".account-hint")).toBeNull();
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

describe("WI-13: AddAccountModal accessibility", () => {
  it("AddAccountModal has role=dialog", async () => {
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
    expect(dialog).toHaveAttribute("aria-labelledby", "add-account-title");
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

    expect(
      await screen.findByText("newly-added@example.com"),
    ).toBeInTheDocument();
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
    expect(document.querySelector(".modal input")).toBeNull();
    expect(
      screen.queryByRole("button", { name: /from refresh token/i }),
    ).toBeNull();
    const modal = document.querySelector(".modal");
    expect(modal?.textContent ?? "").not.toMatch(/paste|sk-ant-ort01/i);
  });
});
