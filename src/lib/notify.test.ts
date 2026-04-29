import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";

const sendNotificationMock = vi.fn();
const isPermissionGrantedMock = vi.fn();
const requestPermissionMock = vi.fn();

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: (...args: unknown[]) => isPermissionGrantedMock(...args),
  requestPermission: (...args: unknown[]) => requestPermissionMock(...args),
  sendNotification: (...args: unknown[]) => sendNotificationMock(...args),
}));

import {
  __bucketsSizeForTests,
  __resetForTests,
  __targetQueueSizeForTests,
  consumeRecentTarget,
  dispatchOsNotification,
  getPermissionStatus,
  requestNotificationPermission,
  subscribePermissionStatus,
  type NotificationTarget,
} from "./notify";

describe("lib/notify — singleton dispatcher", () => {
  beforeEach(() => {
    sendNotificationMock.mockReset();
    isPermissionGrantedMock.mockReset().mockResolvedValue(true);
    requestPermissionMock.mockReset().mockResolvedValue("granted");
    __resetForTests();
    // Default: window unfocused so the focus gate is permissive.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    vi.useRealTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("focus gate", () => {
    it("suppresses dispatch when window has focus", async () => {
      (document.hasFocus as ReturnType<typeof vi.fn>).mockReturnValue(true);
      const ok = await dispatchOsNotification("t", "b");
      expect(ok).toBe(false);
      expect(sendNotificationMock).not.toHaveBeenCalled();
    });

    it("dispatches with ignoreFocus even when focused", async () => {
      (document.hasFocus as ReturnType<typeof vi.fn>).mockReturnValue(true);
      const ok = await dispatchOsNotification("t", "b", { ignoreFocus: true });
      expect(ok).toBe(true);
      expect(sendNotificationMock).toHaveBeenCalled();
    });
  });

  describe("permission flow", () => {
    it("returns granted after a successful probe", async () => {
      // First call triggers the probe; subscribe to observe state.
      const observed: string[] = [];
      const unsub = subscribePermissionStatus((s) => observed.push(s));
      // Wait for probe to resolve.
      await new Promise((r) => setTimeout(r, 0));
      await new Promise((r) => setTimeout(r, 0));
      expect(observed).toContain("granted");
      expect(getPermissionStatus()).toBe("granted");
      unsub();
    });

    it("requests permission on first dispatch when not-requested", async () => {
      isPermissionGrantedMock.mockResolvedValue(false);
      requestPermissionMock.mockResolvedValue("granted");
      // Force the cached state into a known shape via getPermissionStatus()
      // followed by the probe round-trip.
      getPermissionStatus();
      await new Promise((r) => setTimeout(r, 0));
      expect(getPermissionStatus()).toBe("not-requested");

      const ok = await dispatchOsNotification("t", "b");
      expect(ok).toBe(true);
      expect(requestPermissionMock).toHaveBeenCalledTimes(1);
      expect(sendNotificationMock).toHaveBeenCalled();
    });

    it("only prompts once for concurrent first-time dispatches", async () => {
      isPermissionGrantedMock.mockResolvedValue(false);
      // Slow-resolving prompt to expose the race.
      requestPermissionMock.mockImplementation(
        () =>
          new Promise((resolve) =>
            setTimeout(() => resolve("granted"), 10),
          ) as Promise<"granted">,
      );
      getPermissionStatus();
      await new Promise((r) => setTimeout(r, 0));
      const a = dispatchOsNotification("a", "1", { dedupeKey: "x" });
      const b = dispatchOsNotification("b", "2", { dedupeKey: "y" });
      await Promise.all([a, b]);
      // In-flight guard memoizes the prompt: two concurrent dispatches
      // share one OS prompt and one resolve. Both still send their
      // notifications because they share the granted result.
      expect(requestPermissionMock).toHaveBeenCalledTimes(1);
      expect(sendNotificationMock).toHaveBeenCalledTimes(2);
    });

    it("treats a probe failure as retryable (not terminal denied)", async () => {
      // First probe: throws → cache should stay retryable, not flip
      // to "denied". A subsequent probe must re-attempt.
      isPermissionGrantedMock
        .mockRejectedValueOnce(new Error("transient plugin error"))
        .mockResolvedValueOnce(true);
      const observed: string[] = [];
      const unsub = subscribePermissionStatus((s) => observed.push(s));
      await new Promise((r) => setTimeout(r, 0));
      await new Promise((r) => setTimeout(r, 0));
      // After the failed first probe, status must NOT be "denied" —
      // a transient probe error should leave room for a retry.
      expect(getPermissionStatus()).not.toBe("denied");
      // Calling getPermissionStatus again should kick off a fresh
      // probe that this time succeeds → "granted".
      await new Promise((r) => setTimeout(r, 0));
      await new Promise((r) => setTimeout(r, 0));
      expect(observed.includes("granted")).toBe(true);
      unsub();
    });

    it("subscribers fire on status change after request", async () => {
      isPermissionGrantedMock.mockResolvedValue(false);
      requestPermissionMock.mockResolvedValue("granted");
      const observed: string[] = [];
      subscribePermissionStatus((s) => observed.push(s));
      await new Promise((r) => setTimeout(r, 0));
      await new Promise((r) => setTimeout(r, 0));
      // Initial probe → "not-requested"
      expect(observed[observed.length - 1]).toBe("not-requested");
      await requestNotificationPermission();
      // Final state → "granted"
      expect(observed[observed.length - 1]).toBe("granted");
    });
  });

  describe("token bucket coalescing", () => {
    it("admits the first N dispatches and drops the rest in window", async () => {
      const key = "test-key";
      const results: boolean[] = [];
      // 5 dispatches with maxBurst=3 → first 3 pass, rest drop.
      for (let i = 0; i < 5; i++) {
        // eslint-disable-next-line no-await-in-loop
        results.push(
          await dispatchOsNotification("t", `${i}`, {
            dedupeKey: key,
            maxBurst: 3,
            windowMs: 60_000,
          }),
        );
      }
      expect(results).toEqual([true, true, true, false, false]);
      expect(sendNotificationMock).toHaveBeenCalledTimes(3);
    });

    it("uses default 3-in-60s when maxBurst not specified", async () => {
      for (let i = 0; i < 4; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("t", `${i}`, { dedupeKey: "default" });
      }
      // Default maxBurst=3 → 4th dispatch drops.
      expect(sendNotificationMock).toHaveBeenCalledTimes(3);
    });

    it("isolates buckets by dedupeKey", async () => {
      // Two keys with maxBurst=2: one busy session can't starve another.
      for (let i = 0; i < 3; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("a", "1", {
          dedupeKey: "k1",
          maxBurst: 2,
        });
      }
      for (let i = 0; i < 3; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("b", "2", {
          dedupeKey: "k2",
          maxBurst: 2,
        });
      }
      // 4 total: 2 from each key (3rd in each is dropped).
      expect(sendNotificationMock).toHaveBeenCalledTimes(4);
    });

    it("bucket evicts expired stamps after window passes", async () => {
      const tiny = 50; // 50 ms window so test is fast
      for (let i = 0; i < 3; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("t", `${i}`, {
          dedupeKey: "expire",
          maxBurst: 2,
          windowMs: tiny,
        });
      }
      expect(sendNotificationMock).toHaveBeenCalledTimes(2);
      // Wait past the window then dispatch again — should pass.
      await new Promise((r) => setTimeout(r, tiny + 5));
      const ok = await dispatchOsNotification("late", "x", {
        dedupeKey: "expire",
        maxBurst: 2,
        windowMs: tiny,
      });
      expect(ok).toBe(true);
      expect(sendNotificationMock).toHaveBeenCalledTimes(3);
    });

    it("dispatches without dedupeKey are not rate-limited", async () => {
      for (let i = 0; i < 5; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("t", `${i}`);
      }
      expect(sendNotificationMock).toHaveBeenCalledTimes(5);
    });

    it("evicts unique-per-event keys after their window expires", async () => {
      // useOpDoneNotifications uses dedupeKey = `op:<uuid>` — single
      // shot per key. Without sweeping, every completed op leaks one
      // map entry for the lifetime of the renderer. Verify the sweep
      // actually deletes expired buckets.
      const tiny = 50;
      // Fire 10 unique-key dispatches.
      for (let i = 0; i < 10; i++) {
        // eslint-disable-next-line no-await-in-loop
        await dispatchOsNotification("t", `${i}`, {
          dedupeKey: `op:${i}`,
          windowMs: tiny,
        });
      }
      // All 10 keys present pre-eviction.
      expect(__bucketsSizeForTests()).toBe(10);
      // Wait past the window then fire one more dispatch — the sweep
      // runs on dispatch and should evict all 10 expired keys.
      await new Promise((r) => setTimeout(r, tiny + 5));
      await dispatchOsNotification("late", "x", {
        dedupeKey: "op:fresh",
        windowMs: tiny,
      });
      // Only the fresh key should remain.
      expect(__bucketsSizeForTests()).toBe(1);
    });
  });

  describe("payload metadata", () => {
    it("forwards group and sound when present", async () => {
      await dispatchOsNotification("t", "b", {
        group: "session:s1",
        sound: "default",
      });
      expect(sendNotificationMock).toHaveBeenCalledWith({
        title: "t",
        body: "b",
        group: "session:s1",
        sound: "default",
      });
    });

    it("omits group and sound when not present", async () => {
      await dispatchOsNotification("t", "b");
      expect(sendNotificationMock).toHaveBeenCalledWith({
        title: "t",
        body: "b",
      });
    });
  });

  describe("click-target queue", () => {
    const hostTarget: NotificationTarget = {
      kind: "host",
      session_id: "s1",
      cwd: "/home/u/foo",
    };
    const appTarget: NotificationTarget = {
      kind: "app",
      route: { section: "accounts", email: "a@b" },
    };

    it("returns null when nothing has been dispatched", () => {
      expect(consumeRecentTarget()).toBeNull();
      expect(__targetQueueSizeForTests()).toBe(0);
    });

    it("records a target only when the dispatch is accepted", async () => {
      await dispatchOsNotification("t", "b", { target: hostTarget });
      expect(__targetQueueSizeForTests()).toBe(1);
      expect(consumeRecentTarget()).toEqual(hostTarget);
      expect(__targetQueueSizeForTests()).toBe(0);
    });

    it("does not record a target when focus-gated", async () => {
      (document.hasFocus as ReturnType<typeof vi.fn>).mockReturnValue(true);
      await dispatchOsNotification("t", "b", { target: hostTarget });
      expect(__targetQueueSizeForTests()).toBe(0);
      expect(consumeRecentTarget()).toBeNull();
    });

    it("does not record a target when rate-limited", async () => {
      const opts = {
        target: hostTarget,
        dedupeKey: "k",
        maxBurst: 2,
      } as const;
      await dispatchOsNotification("t", "1", opts);
      await dispatchOsNotification("t", "2", opts);
      // Third dispatch is dropped by the bucket.
      const ok = await dispatchOsNotification("t", "3", opts);
      expect(ok).toBe(false);
      // Two targets queued — the dropped dispatch left no trace.
      expect(__targetQueueSizeForTests()).toBe(2);
    });

    it("consumes the most recent target and clears older ones", async () => {
      // Two dispatches; the newer banner is what the user clicked on.
      // Older queued targets belong to dismissed banners and must be
      // dropped — otherwise the next focus event would route to them.
      await dispatchOsNotification("t", "1", { target: hostTarget });
      await dispatchOsNotification("t", "2", { target: appTarget });
      expect(consumeRecentTarget()).toEqual(appTarget);
      expect(consumeRecentTarget()).toBeNull();
    });

    it("expires entries after the 10 s TTL", async () => {
      // Advance time past the TTL by stubbing Date.now via the
      // dispatcher's `now` reads. Vitest fake timers don't shim
      // Date.now in this codebase's harness; spy directly.
      const realNow = Date.now;
      const baseline = 1_000_000;
      let current = baseline;
      vi.spyOn(Date, "now").mockImplementation(() => current);

      await dispatchOsNotification("t", "b", { target: hostTarget });
      expect(__targetQueueSizeForTests()).toBe(1);

      // 11 s later — past the 10 s TTL.
      current = baseline + 11_000;
      expect(consumeRecentTarget()).toBeNull();
      // The next push should evict the expired entry too.
      await dispatchOsNotification("t", "b", { target: appTarget });
      expect(__targetQueueSizeForTests()).toBe(1);

      Date.now = realNow;
    });

    it("preserves queue across reset only via __resetForTests", async () => {
      await dispatchOsNotification("t", "b", { target: hostTarget });
      expect(__targetQueueSizeForTests()).toBe(1);
      __resetForTests();
      expect(__targetQueueSizeForTests()).toBe(0);
    });
  });
});
