import { describe, expect, it } from "vitest";

/**
 * The double-spend guard, extracted as pure state transitions.
 *
 * `handleWake` lives inside AccountsSection and needs the whole app
 * provider tree to render, so the guard is exercised here as the state
 * machine it is. This is the one action in the app that spends the
 * user's money, and the original implementation had no guard at all:
 * a single `wakingUuid` string, cleared in a `finally` that ran the
 * moment the request resolved. Two failure modes followed from that,
 * both covered below.
 */

/** Mirrors the `setWaking` reducer in AccountsSection.handleWake. */
function claim(prev: ReadonlySet<string>, uuid: string): {
  next: ReadonlySet<string>;
  started: boolean;
} {
  if (prev.has(uuid)) return { next: prev, started: false };
  return { next: new Set(prev).add(uuid), started: true };
}

function release(prev: ReadonlySet<string>, uuid: string): ReadonlySet<string> {
  const next = new Set(prev);
  next.delete(uuid);
  return next;
}

describe("wake double-spend guard", () => {
  it("lets the first click through", () => {
    const { started } = claim(new Set(), "a");
    expect(started).toBe(true);
  });

  it("refuses a second click while the first is in flight", () => {
    // Without this, every extra click is another billable request.
    const first = claim(new Set(), "a");
    const second = claim(first.next, "a");
    expect(second.started).toBe(false);
    expect(second.next).toBe(first.next); // identity preserved — no re-render
  });

  it("keeps other accounts independently wakeable", () => {
    // A Set, not a single uuid: waking account A must not block B.
    const a = claim(new Set(), "a");
    const b = claim(a.next, "b");
    expect(b.started).toBe(true);
    expect([...b.next].sort()).toEqual(["a", "b"]);
  });

  it("does not let one account's completion unblock another", () => {
    // The single-uuid bug: finishing A cleared the flag entirely, so a
    // still-in-flight B became clickable again.
    const a = claim(new Set(), "a");
    const b = claim(a.next, "b");
    const afterA = release(b.next, "a");
    expect(afterA.has("b")).toBe(true);
    expect(claim(afterA, "b").started).toBe(false);
  });

  it("re-allows a wake only after release", () => {
    // Release happens on the delayed refresh, not at request-end —
    // `needsWake` stays true until usage repaints, so releasing early
    // would re-enable the menu item for the whole 25s window.
    const first = claim(new Set(), "a");
    const freed = release(first.next, "a");
    expect(claim(freed, "a").started).toBe(true);
  });

  it("releases on failure so a failed wake can be retried", () => {
    const first = claim(new Set(), "a");
    const freed = release(first.next, "a"); // catch-branch release
    expect(freed.has("a")).toBe(false);
  });

  it("stays claimed until the refresh settles, not when it starts", () => {
    // Verification caught this: releasing when refreshUsageFor was
    // *called* left a gap where usage had not repainted yet, so
    // needsWake was still true and the item was clickable again.
    let held: ReadonlySet<string> = claim(new Set(), "a").next;
    const refresh = Promise.resolve().then(() => {
      // mid-refresh: still claimed
      expect(held.has("a")).toBe(true);
    });
    return refresh
      .finally(() => {
        held = release(held, "a");
      })
      .then(() => {
        expect(held.has("a")).toBe(false);
      });
  });
});

/** Mirrors `errorText` in AccountsSection. */
function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return msg;
    try {
      return JSON.stringify(err);
    } catch {
      return "unknown error";
    }
  }
  return String(err);
}

describe("errorText", () => {
  it("never renders [object Object]", () => {
    // The first fix used String(err), which produces exactly that for a
    // plain object — verification caught it as NOT FIXED.
    for (const err of [
      { code: 429 },
      { nested: { a: 1 } },
      Object.create(null),
    ]) {
      expect(errorText(err)).not.toBe("[object Object]");
    }
  });

  it("prefers a message field when present", () => {
    expect(errorText({ message: "token unavailable" })).toBe(
      "token unavailable",
    );
  });

  it("passes Error and string through", () => {
    expect(errorText(new Error("boom"))).toBe("boom");
    // Tauri rejects with a plain string for Result<_, String> commands.
    expect(errorText("no credentials stored for a@b.com")).toBe(
      "no credentials stored for a@b.com",
    );
  });

  it("survives a circular object", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    expect(errorText(circular)).toBe("unknown error");
  });
});
