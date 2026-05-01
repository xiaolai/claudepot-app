// Focus-gate logging contract: dispatchOsNotification must persist
// the notification log entry even when the OS banner is suppressed
// by the focus gate.
//
// Pre-fix, a focused user got NOTHING — no toast (the dispatcher's
// only surface is OS), no banner (focus-gated), no log entry. Usage-
// threshold notifications could silently miss the user entirely.
// Now the log catches the intent regardless; the bell badge surfaces
// it within ~8 s (or immediately via the same-window event).

import { describe, expect, it, vi, beforeEach } from "vitest";

type InvokeArgs = [cmd: string, args?: unknown];
const invokeSpy = vi.fn<(...args: InvokeArgs) => Promise<unknown>>(
  async () => undefined,
);
const sendNotificationSpy = vi.fn(() => undefined);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: InvokeArgs) => invokeSpy(...args),
}));

// Permission helpers default to "granted" so the focus gate is the
// only thing under test. Each test can override either.
const isPermissionGrantedMock = vi.fn(async () => true);
const requestPermissionMock = vi.fn(async () => "granted" as string);
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => isPermissionGrantedMock(),
  requestPermission: () => requestPermissionMock(),
  sendNotification: (...args: unknown[]) => sendNotificationSpy(...(args as [])),
}));

import { __resetForTests, dispatchOsNotification } from "./notify";

function setFocus(focused: boolean) {
  // jsdom defaults `document.hasFocus()` to false. Override per-test
  // so we can drive both branches deterministically.
  Object.defineProperty(document, "hasFocus", {
    value: () => focused,
    configurable: true,
  });
}

describe("dispatchOsNotification — focus-gate logging", () => {
  beforeEach(() => {
    invokeSpy.mockClear();
    invokeSpy.mockImplementation(async () => undefined);
    sendNotificationSpy.mockClear();
    isPermissionGrantedMock.mockClear();
    isPermissionGrantedMock.mockResolvedValue(true);
    requestPermissionMock.mockClear();
    requestPermissionMock.mockResolvedValue("granted");
    __resetForTests();
  });

  it("logs even when focus suppresses the OS banner", async () => {
    setFocus(true);
    const result = await dispatchOsNotification("Switched", "to bob@x.com", {
      kind: "info",
    });
    expect(result).toBe(false);
    expect(sendNotificationSpy).not.toHaveBeenCalled();

    const logCalls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(logCalls).toHaveLength(1);
    expect(logCalls[0][1]).toEqual({
      args: {
        source: "os",
        kind: "info",
        title: "Switched",
        body: "to bob@x.com",
        target: null,
      },
    });
  });

  it("logs AND fires when window is unfocused", async () => {
    setFocus(false);
    const result = await dispatchOsNotification("Switched", "to bob@x.com");
    expect(result).toBe(true);
    expect(sendNotificationSpy).toHaveBeenCalledOnce();

    const logCalls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(logCalls).toHaveLength(1);
  });

  it("logs even when permission denied", async () => {
    // The OS banner won't fire because permission is denied, but the
    // log must still record the intent. Permission denial is "no OS
    // notifications," not "no in-app history."
    //
    // Setup: probe returns false (not granted yet) AND request returns
    // "denied" (user said no). The dispatcher's own state machine
    // promotes that to cached === "denied", which suppresses the OS
    // send. The log write must still have happened up-front.
    setFocus(false);
    isPermissionGrantedMock.mockResolvedValue(false);
    requestPermissionMock.mockResolvedValue("denied");
    const result = await dispatchOsNotification("Repair", "complete");
    expect(result).toBe(false);
    expect(sendNotificationSpy).not.toHaveBeenCalled();

    const logCalls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(logCalls).toHaveLength(1);
  });

  it("default kind is 'notice' for OS dispatches without an explicit kind", async () => {
    setFocus(true);
    await dispatchOsNotification("Auth rejected", "Sign in again");
    const logCalls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    const payload = logCalls[0][1] as { args: { kind: string } };
    expect(payload.args.kind).toBe("notice");
  });

  it("ignoreFocus bypasses the focus gate for fatal-class alerts", async () => {
    setFocus(true);
    const result = await dispatchOsNotification("Keychain locked", "Unlock", {
      ignoreFocus: true,
      kind: "error",
    });
    expect(result).toBe(true);
    expect(sendNotificationSpy).toHaveBeenCalledOnce();
    const logCalls = invokeSpy.mock.calls.filter(
      (c) => c[0] === "notification_log_append",
    );
    expect(logCalls).toHaveLength(1);
  });
});
