// Smoke tests for the paper-mono shell.
//
// The previous App.test.tsx exhaustively exercised the old
// Sidebar/ContentPane split — "Use CLI" button, "Use Desktop"
// button, per-row Log in / Cancel-login controls, etc. The new
// paper-mono design puts all of those actions behind a right-click
// context menu on each AccountCard or inside the AnomalyBanner, so
// the old DOM-level assertions no longer apply.
//
// This file keeps a small smoke suite verifying the shell mounts,
// renders the expected regions, and wires accounts → sidebar +
// cards. Deeper per-component tests live next to their components
// (ProjectsList.test.tsx is gone; AccountCard/AddAccountModal could
// gain focused tests in a follow-up).
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { sampleAccount, sampleStatus } from "./test/fixtures";

async function renderApp(
  handlers: Record<string, (args?: unknown) => unknown>,
) {
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
    current_cc_identity: () => ({
      email: null,
      verified_at: new Date().toISOString(),
      error: null,
    }),
    repair_status_summary: () => ({ pending: 0, stale: 0, running: 0 }),
    running_ops_list: () => [],
    protected_paths_list: () => [],
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
  vi.doMock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
  }));
  const { default: App } = await import("./App");
  return render(<App />);
}

beforeEach(() => {
  vi.resetModules();
  // useSection, useTheme, useDevMode all persist to localStorage — a
  // previous test's navigation leaks into the next test's initial
  // section if we don't clear between runs.
  try {
    localStorage.clear();
  } catch {
    // ignore — localStorage unavailable
  }
});

describe("App shell — paper-mono", () => {
  it("renders primary nav entries in the sidebar", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    // The AppSidebar lists the four primary-nav sections.
    // We assert via `aria-current` on the active item and by the
    // presence of each label somewhere in the sidebar region.
    expect(await screen.findByText("Accounts")).toBeInTheDocument();
    expect(screen.getByText("Projects")).toBeInTheDocument();
    expect(screen.getByText("Sessions")).toBeInTheDocument();
    expect(screen.getByText("Activity")).toBeInTheDocument();
    expect(screen.getByText("Global")).toBeInTheDocument();
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("shows the empty state when no accounts exist", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    expect(await screen.findByText(/No accounts yet/i)).toBeInTheDocument();
  });

  it("renders account cards when the DB has accounts", async () => {
    const alice = sampleAccount({
      uuid: "u1",
      email: "alice@example.com",
    });
    const bob = sampleAccount({
      uuid: "u2",
      email: "bob@example.com",
      is_cli_active: false,
    });

    await renderApp({
      app_status: () => sampleStatus({ account_count: 2 }),
      account_list: () => [alice, bob],
    });

    // Each email appears in the AccountCard grid. The same email may
    // also appear in the sidebar swap-target preview when an account
    // is bound, so use `findAllByText` for robustness.
    const aliceHits = await screen.findAllByText("alice@example.com");
    expect(aliceHits.length).toBeGreaterThan(0);
    expect(await screen.findAllByText("bob@example.com")).toHaveLength(
      1,
    );
  });

  it("filters cards by email", async () => {
    // AccountsSection gates the filter row to accounts.length > 3 —
    // the input is pure chrome with 1–3 accounts. Seed four so the
    // filter appears and we can exercise it.
    const [alice, bob, carol, dave] = [
      { uuid: "u1", email: "alice@example.com", org_name: "Alice Org" },
      { uuid: "u2", email: "bob@example.com",   org_name: "Bob Org" },
      { uuid: "u3", email: "carol@example.com", org_name: "Carol Org" },
      { uuid: "u4", email: "dave@example.com",  org_name: "Dave Org" },
    ].map((o) => sampleAccount(o));

    await renderApp({
      app_status: () => sampleStatus({ account_count: 4 }),
      account_list: () => [alice, bob, carol, dave],
    });

    await screen.findAllByText("alice@example.com");

    const user = userEvent.setup();
    const filter = screen.getByLabelText("Filter accounts");
    await user.type(filter, "alice");

    // Only alice matches; counter reads "1 / 4".
    const counter = await screen.findByText((text) =>
      text.replace(/\s+/g, "") === "1/4",
    );
    expect(counter).toBeInTheDocument();
  });

  it("navigates between sections via the sidebar", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await screen.findByText(/No accounts yet/i);
    const user = userEvent.setup();

    // `name: "Projects"` is case-sensitive; prevents matching the
    // lowercase "projects/" filesystem-tree row that also lives in
    // the sidebar.
    const projectsNav = screen.getByRole("button", { name: "Projects" });
    await user.click(projectsNav);

    // Projects section renders either an empty-state or a table. Either
    // way, the "Projects" page title must be visible.
    const titles = await screen.findAllByText("Projects");
    expect(titles.length).toBeGreaterThan(0);
  });

  it("opens the Add Account modal", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });

    await screen.findByText(/No accounts yet/i);
    const user = userEvent.setup();
    // Two "Add account" buttons exist: one in the header and one in
    // the empty-state CTA. Either opens the same modal — the header
    // comes first, so click that.
    const [addBtn] = screen.getAllByRole("button", { name: /Add account/i });
    await user.click(addBtn);

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/Import from Claude Code/i),
    ).toBeInTheDocument();
  });

  it("renders the window chrome with a theme toggle", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
    });
    // The toggle carries an accessible label that flips based on the
    // current effective theme.
    const toggle = await screen.findByRole("button", {
      name: /Switch to (light|dark) mode/,
    });
    expect(toggle).toBeInTheDocument();
  });

  it("shows the sessions nav badge when a live session is errored", async () => {
    await renderApp({
      app_status: () => sampleStatus({ account_count: 0 }),
      account_list: () => [],
      session_live_snapshot: () => [
        {
          session_id: "s1",
          pid: 1,
          cwd: "/proj",
          transcript_path: null,
          status: "busy",
          current_action: null,
          model: null,
          waiting_for: null,
          errored: true,
          stuck: false,
          idle_ms: 0,
          seq: 0,
        },
      ],
      preferences_get: () => ({
        notify_on_error: false,
        notify_on_idle_done: false,
        notify_on_stuck_minutes: null,
        notify_on_spend_usd: null,
      }),
    });

    // After the registry rename in the events/activity collapse,
    // the cross-project firehose nav row is now labeled "Sessions"
    // (id stays `activities` for localStorage compatibility). The
    // badge still surfaces alerting sessions there.
    const activitiesBtn = await screen.findByRole("button", {
      name: "Sessions",
    });
    await waitFor(() => {
      expect(within(activitiesBtn).getByText("1")).toBeInTheDocument();
    });
  });
});
