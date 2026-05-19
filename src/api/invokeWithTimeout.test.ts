import { describe, expect, it, vi, beforeEach } from "vitest";

// The helper imports `invoke` from `@tauri-apps/api/core`. The mock
// stays in scope for the entire file — each test resets the spy to a
// fresh impl in `beforeEach`.
const invokeSpy = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invokeSpy(...a),
}));

import { invokeWithTimeout, IpcTimeoutError } from "./invokeWithTimeout";

describe("invokeWithTimeout", () => {
  beforeEach(() => {
    invokeSpy.mockReset();
  });

  it("resolves with the IPC result when invoke wins the race", async () => {
    invokeSpy.mockResolvedValue("ok");
    const result = await invokeWithTimeout<string>("test_cmd", { a: 1 }, 10_000);
    expect(result).toBe("ok");
    expect(invokeSpy).toHaveBeenCalledWith("test_cmd", { a: 1 });
  });

  it("rejects with IpcTimeoutError when the budget expires before invoke resolves", async () => {
    // Use real (but tiny) timers so we don't fight vitest's fake-
    // timer interaction with promise microtasks — 20ms is plenty
    // for the race to settle deterministically while the test stays
    // fast.
    invokeSpy.mockImplementation(
      // The invoke side never settles; the timeout side wins.
      () => new Promise<string>(() => {}),
    );
    const err = await invokeWithTimeout<string>("slow_cmd", undefined, 20).catch(
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(IpcTimeoutError);
    expect(err).toMatchObject({ command: "slow_cmd", ms: 20 });
  });

  it("propagates IPC errors verbatim — does not wrap them in IpcTimeoutError", async () => {
    invokeSpy.mockRejectedValue(new Error("Rust side blew up"));
    const err = await invokeWithTimeout<string>(
      "bad_cmd",
      undefined,
      10_000,
    ).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe("Rust side blew up");
    // The .name guard catches structural shadowing if both error
    // types ever share a base.
    expect(err).not.toBeInstanceOf(IpcTimeoutError);
  });

  it("clears its internal timer on success — no lingering setTimeout after resolution", async () => {
    invokeSpy.mockResolvedValue("done");
    const clearSpy = vi.spyOn(globalThis, "clearTimeout");
    await invokeWithTimeout<string>("fast_cmd", undefined, 60_000);
    expect(clearSpy).toHaveBeenCalled();
    clearSpy.mockRestore();
  });
});
