import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Module-level handler captured per-test so the mocked `listen` can
// inject synthetic `op-progress::<op_id>` events.
let capturedHandler:
  | ((event: { payload: unknown }) => void)
  | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_channel: string, h: (e: { payload: unknown }) => void) => {
    capturedHandler = h;
    return () => {
      capturedHandler = null;
    };
  }),
  // runVerifyAll emits `usage::refetch` on successful completion so the
  // Accounts screen re-pulls usage after a heal.
  emit: vi.fn(async () => {}),
}));

vi.mock("../../api", () => ({
  api: {
    verifyAllAccountsStart: vi.fn(async () => "op-fake"),
    accountList: vi.fn(async () => [
      {
        uuid: "u1",
        email: "alice@example.com",
        verify_status: "ok",
        verified_email: "alice@example.com",
      },
      {
        uuid: "u2",
        email: "bob@example.com",
        verify_status: "drift",
        verified_email: "elsewhere@example.com",
      },
    ]),
  },
}));

import { emit } from "@tauri-apps/api/event";
import { USAGE_REFETCH_EVENT } from "../../lib/events";
import { runVerifyAll } from "./runVerifyAll";

beforeEach(() => {
  capturedHandler = null;
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("runVerifyAll", () => {
  it("patches rows in order and re-fetches on terminal complete", async () => {
    const patches: { uuid: string; patch: Record<string, unknown> }[] = [];
    const setRows = vi.fn();

    const promise = runVerifyAll({
      patchAccount: (uuid, patch) =>
        patches.push({ uuid, patch: patch as Record<string, unknown> }),
      setAccounts: setRows,
    });

    // Yield once so `listen` resolves and `capturedHandler` is set.
    // The mock returns synchronously so a single tick is enough.
    await Promise.resolve();
    await Promise.resolve();
    expect(capturedHandler).not.toBeNull();
    const fire = capturedHandler!;

    // Per-account events: first OK, then drift.
    fire({
      payload: {
        op_id: "op-fake",
        kind: "verify_account",
        uuid: "u1",
        email: "alice@example.com",
        idx: 1,
        total: 2,
        outcome: "ok",
      },
    });
    fire({
      payload: {
        op_id: "op-fake",
        kind: "verify_account",
        uuid: "u2",
        email: "bob@example.com",
        idx: 2,
        total: 2,
        outcome: "drift",
        detail: "actual: elsewhere@example.com",
      },
    });
    // Terminal `op` event.
    fire({
      payload: {
        op_id: "op-fake",
        phase: "op",
        status: "complete",
      },
    });

    const summary = await promise;
    expect(summary).toEqual({
      total: 2,
      ok: 1,
      drift: 1,
      rejected: 0,
      network_error: 0,
    });

    // Two patches in order, OK first then drift.
    expect(patches).toHaveLength(2);
    expect(patches[0].uuid).toBe("u1");
    expect(patches[0].patch).toEqual({ verify_status: "ok" });
    expect(patches[1].uuid).toBe("u2");
    expect(patches[1].patch).toMatchObject({
      verify_status: "drift",
      verified_email: "elsewhere@example.com",
    });

    // setAccounts called once at the end with the refreshed list.
    expect(setRows).toHaveBeenCalledTimes(1);
    expect(setRows.mock.calls[0][0]).toHaveLength(2);
  });

  it("ignores events with mismatched op_id", async () => {
    const patches: { uuid: string }[] = [];
    const promise = runVerifyAll({
      patchAccount: (uuid) => patches.push({ uuid }),
      setAccounts: () => {},
    });
    await Promise.resolve();
    await Promise.resolve();
    const fire = capturedHandler!;

    fire({
      payload: {
        op_id: "wrong-op-id",
        kind: "verify_account",
        uuid: "u1",
        email: "x",
        idx: 1,
        total: 1,
        outcome: "ok",
      },
    });
    fire({
      payload: {
        op_id: "op-fake",
        phase: "op",
        status: "complete",
      },
    });
    await promise;
    expect(patches).toHaveLength(0);
  });

  it("emits usage::refetch on a clean terminal so usage re-pulls", async () => {
    const promise = runVerifyAll({
      patchAccount: () => {},
      setAccounts: () => {},
    });
    await Promise.resolve();
    await Promise.resolve();
    capturedHandler!({
      payload: { op_id: "op-fake", phase: "op", status: "complete" },
    });
    await promise;
    expect(vi.mocked(emit)).toHaveBeenCalledWith(USAGE_REFETCH_EVENT);
  });

  it("does NOT emit usage::refetch when the op terminates with an error", async () => {
    const promise = runVerifyAll({
      patchAccount: () => {},
      setAccounts: () => {},
    });
    await Promise.resolve();
    await Promise.resolve();
    capturedHandler!({
      payload: {
        op_id: "op-fake",
        phase: "op",
        status: "error",
        detail: "boom",
      },
    });
    await expect(promise).rejects.toThrow(/boom/);
    expect(vi.mocked(emit)).not.toHaveBeenCalled();
  });
});
