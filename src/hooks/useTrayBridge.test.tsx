import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import type { AccountSummary } from "../types";
import { sampleAccount } from "../test/fixtures";

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

const traySetAlertCount = vi.fn();
const sessionLiveSnapshot = vi.fn();
vi.mock("../api", () => ({
  api: {
    traySetAlertCount: (...a: unknown[]) => traySetAlertCount(...a),
    sessionLiveSnapshot: (...a: unknown[]) => sessionLiveSnapshot(...a),
  },
}));

import { useTrayBridge } from "./useTrayBridge";

type Listener = (ev: { payload?: unknown }) => void;

function setupBus() {
  const listeners = new Map<string, Listener>();
  listenMock.mockImplementation(async (channel: string, fn: Listener) => {
    listeners.set(channel, fn);
    return () => listeners.delete(channel);
  });
  return {
    fire: (channel: string, payload: unknown) =>
      listeners.get(channel)?.({ payload }),
    has: (channel: string) => listeners.has(channel),
  };
}

type UseCliMock = ReturnType<
  typeof vi.fn<(a: AccountSummary, force?: boolean) => Promise<void>>
>;
type PushToastMock = ReturnType<
  typeof vi.fn<(kind: "info" | "error", text: string) => void>
>;

function makeUseCli(): UseCliMock {
  return vi.fn<(a: AccountSummary, force?: boolean) => Promise<void>>(
    async () => undefined,
  );
}

// Keep every field a vi.fn() mock (no Partial<TrayArgs> spread — it
// would widen `emit` to the EmitFn union and lose `.mock` access).
function makeArgs(
  overrides: {
    alertCount?: number;
    accounts?: AccountSummary[];
    actions?: { useCli: UseCliMock };
    pushToast?: PushToastMock;
  } = {},
) {
  return {
    alertCount: overrides.alertCount ?? 0,
    setSection: vi.fn(),
    setPendingSessionPath: vi.fn(),
    setPendingProjectPath: vi.fn(),
    requestDesktopSignOut: vi.fn(),
    accounts: overrides.accounts ?? ([] as AccountSummary[]),
    actions: overrides.actions ?? { useCli: makeUseCli() },
    pushToast: overrides.pushToast ?? vi.fn(),
    emit: vi.fn().mockResolvedValue({}),
    refreshAccounts: vi.fn().mockResolvedValue(undefined),
  };
}

beforeEach(() => {
  listenMock.mockReset();
  traySetAlertCount.mockReset().mockResolvedValue(undefined);
  sessionLiveSnapshot.mockReset().mockResolvedValue([]);
});

describe("useTrayBridge — tray badge mirror", () => {
  it("fires the IPC only when the count actually changes", () => {
    setupBus();
    const { rerender } = renderHook(
      ({ count }: { count: number }) =>
        useTrayBridge(makeArgs({ alertCount: count })),
      { initialProps: { count: 0 } },
    );
    expect(traySetAlertCount).toHaveBeenCalledTimes(1);
    expect(traySetAlertCount).toHaveBeenCalledWith(0);

    rerender({ count: 0 });
    expect(traySetAlertCount).toHaveBeenCalledTimes(1); // unchanged → no IPC

    rerender({ count: 2 });
    expect(traySetAlertCount).toHaveBeenCalledTimes(2);
    expect(traySetAlertCount).toHaveBeenLastCalledWith(2);
  });
});

describe("useTrayBridge — subscriptions", () => {
  it("wires all four tray channels once for the shell's lifetime", async () => {
    const bus = setupBus();
    const { rerender } = renderHook(
      ({ accounts }: { accounts: AccountSummary[] }) =>
        useTrayBridge(makeArgs({ accounts })),
      { initialProps: { accounts: [] as AccountSummary[] } },
    );
    await Promise.resolve();

    for (const ch of [
      "cp-activity-open-session",
      "cp-tray-desktop-clear",
      "cp-tray-desktop-bind",
      "tray-cli-switched",
      "tray-cli-switch-failed",
    ]) {
      expect(bus.has(ch), ch).toBe(true);
    }
    const subscribeCalls = listenMock.mock.calls.length;

    // Changing accounts identity (which the switched-handler itself
    // triggers via refreshAccounts) must NOT re-subscribe any channel.
    rerender({ accounts: [sampleAccount({ uuid: "u1" })] });
    await Promise.resolve();
    expect(listenMock.mock.calls.length).toBe(subscribeCalls);
  });
});

describe("useTrayBridge — tray-cli-switched", () => {
  it("emits an accountSwitched entry with an Undo action", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("tray-cli-switched", {
      to_email: "b@example.com",
      from_email: "a@example.com",
      cc_was_running: false,
    });

    expect(args.refreshAccounts).toHaveBeenCalled();
    expect(args.emit).toHaveBeenCalledTimes(1);
    const call = args.emit.mock.calls[0][0];
    expect(call.category).toBe("accountSwitched");
    expect(call.title).toBe("CLI → b@example.com");
    expect(call.toastAction?.label).toBe("Undo");
    expect(call.toastAction?.timeoutMs).toBe(10_000);
  });

  it("appends the restart caveat when CC was running", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("tray-cli-switched", {
      to_email: "b@example.com",
      from_email: null,
      cc_was_running: true,
    });
    const call = args.emit.mock.calls[0][0];
    expect(call.title).toContain("restart Claude Code to apply");
    expect(call.toastAction).toBeUndefined(); // no from_email → no Undo
  });

  it("tolerates payload shape drift by refreshing and bailing", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("tray-cli-switched", { wrong: "shape" });
    expect(args.refreshAccounts).toHaveBeenCalledTimes(1);
    expect(args.emit).not.toHaveBeenCalled();
  });

  it("Undo reads the LATEST accounts snapshot at press time", async () => {
    const bus = setupBus();
    const initial = makeArgs({ accounts: [] });
    const { rerender } = renderHook(
      ({ a }: { a: Parameters<typeof useTrayBridge>[0] }) => useTrayBridge(a),
      { initialProps: { a: initial } },
    );
    await Promise.resolve();

    bus.fire("tray-cli-switched", {
      to_email: "b@example.com",
      from_email: "a@example.com",
      cc_was_running: false,
    });
    const undo = initial.emit.mock.calls[0][0].toastAction.onPress as
      () => void;

    // The post-swap refresh lands a fresh accounts list BEFORE the
    // user presses Undo. The undo closure must see it via the ref.
    const prevAccount = sampleAccount({
      uuid: "u-prev",
      email: "a@example.com",
    });
    const updated = makeArgs({
      accounts: [prevAccount],
      // share spies with the original args so assertions see them
      actions: initial.actions,
      pushToast: initial.pushToast,
    });
    rerender({ a: updated });

    undo();
    expect(initial.actions.useCli).toHaveBeenCalledWith(prevAccount, true);
    expect(initial.pushToast).not.toHaveBeenCalled();
  });
});

describe("useTrayBridge — tray-cli-switch-failed", () => {
  it("emits an error entry carrying the failure detail", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("tray-cli-switch-failed", "keychain locked");
    const call = args.emit.mock.calls[0][0];
    expect(call.kind).toBe("error");
    expect(call.title).toBe("CLI switch failed");
    expect(call.body).toBe("keychain locked");
  });

  it("falls back to 'unknown' for an empty payload", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("tray-cli-switch-failed", "");
    expect(args.emit.mock.calls[0][0].body).toBe("unknown");
  });
});

describe("useTrayBridge — tray routing", () => {
  it("cp-activity-open-session resolves the transcript and jumps to projects", async () => {
    const bus = setupBus();
    sessionLiveSnapshot.mockResolvedValue([
      {
        session_id: "s1",
        transcript_path: "/t/s1.jsonl",
        cwd: "/proj",
      },
    ]);
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("cp-activity-open-session", "s1");
    // Let the async snapshot resolution settle.
    await Promise.resolve();
    await Promise.resolve();

    expect(args.setPendingSessionPath).toHaveBeenCalledWith("/t/s1.jsonl");
    expect(args.setPendingProjectPath).toHaveBeenCalledWith("/proj");
    expect(args.setSection).toHaveBeenCalledWith("projects");
  });

  it("cp-tray-desktop-clear routes to the sign-out confirm flow", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("cp-tray-desktop-clear", null);
    expect(args.requestDesktopSignOut).toHaveBeenCalledTimes(1);
  });

  it("cp-tray-desktop-bind routes to Accounts", async () => {
    const bus = setupBus();
    const args = makeArgs();
    renderHook(() => useTrayBridge(args));
    await Promise.resolve();

    bus.fire("cp-tray-desktop-bind", null);
    expect(args.setSection).toHaveBeenCalledWith("accounts");
  });
});
