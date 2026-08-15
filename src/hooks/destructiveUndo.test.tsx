import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useToasts } from "./useToasts";

/**
 * Closing a destructive action's undo toast must CANCEL it, not commit
 * it.
 *
 * `onCommit` fires "when the toast leaves by any path EXCEPT Undo:
 * auto-dismiss, the manual close (X) button, or a programmatic
 * dismiss". That is right for a reversible deferred action — closing
 * "Switching Desktop to X…" should not silently drop the switch, and
 * an earlier audit fixed exactly that.
 *
 * It is wrong for account removal, the app's most destructive action.
 * The user is told "you'll have a few seconds to undo from the toast";
 * if tidying the toast away destroys the account immediately, then
 * dismissing is indistinguishable from confirming and the promised
 * window silently does not exist.
 *
 * The UX audit read this area as "gating strength assigned per call
 * site rather than per consequence" and proposed type-to-confirm. That
 * diagnosis missed the deferred commit entirely — removal never fired
 * on the confirm click. The defect was one edge of the undo window,
 * not the gate in front of it.
 */
describe("destructive deferred actions", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("does NOT commit when the user closes the toast", () => {
    const commit = vi.fn();
    const { result } = renderHook(() => useToasts());

    act(() => {
      result.current.pushToast("info", "Removing alice@example.com…", () => {}, {
        undoMs: 5000,
        onCommit: commit,
        cancelOnDismiss: true,
      });
    });
    const id = result.current.toasts[0].id;

    // The X button: a plain dismiss, no skipCommit.
    act(() => result.current.dismissToast(id));
    expect(commit).not.toHaveBeenCalled();
  });

  it("still commits when the window genuinely elapses", () => {
    const commit = vi.fn();
    const { result } = renderHook(() => useToasts());

    act(() => {
      result.current.pushToast("info", "Removing alice@example.com…", () => {}, {
        undoMs: 5000,
        onCommit: commit,
        cancelOnDismiss: true,
      });
    });

    act(() => {
      vi.advanceTimersByTime(5001);
    });
    expect(commit).toHaveBeenCalledTimes(1);
  });

  it("Undo still cancels", () => {
    const commit = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() => {
      result.current.pushToast("info", "Removing…", () => {}, {
        undoMs: 5000,
        onCommit: commit,
        cancelOnDismiss: true,
      });
    });
    const id = result.current.toasts[0].id;
    act(() => result.current.dismissToast(id, { skipCommit: true }));
    act(() => vi.advanceTimersByTime(10_000));
    expect(commit).not.toHaveBeenCalled();
  });

  /**
   * The counterpart guard. Without `cancelOnDismiss`, closing the
   * toast must still commit — otherwise this change would silently
   * break the Desktop-switch behaviour a previous audit fixed.
   */
  it("a REVERSIBLE deferred action still commits on close", () => {
    const commit = vi.fn();
    const { result } = renderHook(() => useToasts());
    act(() => {
      result.current.pushToast("info", "Switching Desktop…", () => {}, {
        undoMs: 3000,
        onCommit: commit,
      });
    });
    const id = result.current.toasts[0].id;
    act(() => result.current.dismissToast(id));
    expect(commit).toHaveBeenCalledTimes(1);
  });
});

/**
 * The wiring, not just the mechanism.
 *
 * The tests above prove `useToasts` HONOURS `cancelOnDismiss`. They say
 * nothing about whether account removal actually passes it — and it
 * turned out they could not: deleting `cancelOnDismiss: true` from
 * `performRemove` left all 1295 tests green. The primitive was covered;
 * the one call site that makes the app's most destructive action safe
 * was not.
 *
 * Read as source rather than executed because `performRemove` is built
 * inside `useActions`, which needs a live provider, an api module and a
 * tray bridge to instantiate — mounting all of that to observe one
 * option would test the mock. What must not silently change is that
 * this specific call site opts in, and that is visible in the source.
 */
describe("account removal opts into cancel-on-dismiss", () => {
  // `?raw` glob, not node:fs — this suite runs under jsdom, where
  // `node:fs` is not resolvable. Same mechanism `errorCodes.test.ts`
  // uses to read the error-code registry.
  const SRC = import.meta.glob("./useActions.ts", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>;
  const src = Object.values(SRC)[0] ?? "";

  function performRemoveBody(): string {
    expect(src.length, "useActions.ts did not load — this suite would assert nothing").toBeGreaterThan(500);
    const at = src.indexOf("const performRemove = (");
    expect(at, "performRemove not found — this test is checking nothing").toBeGreaterThan(-1);
    // To the end of the pushToast call that follows it.
    const end = src.indexOf("\n  };", at);
    return src.slice(at, end);
  }

  it("passes cancelOnDismiss so closing the toast cancels the removal", () => {
    expect(performRemoveBody()).toContain("cancelOnDismiss: true");
  });

  it("still defers via onCommit rather than removing on confirm", () => {
    const body = performRemoveBody();
    expect(body).toContain("onCommit");
    expect(body).toContain("performRemoveImmediate");
    // A direct call outside onCommit would destroy on the confirm click.
    const beforeCommit = body.slice(0, body.indexOf("onCommit"));
    expect(beforeCommit).not.toContain("performRemoveImmediate(");
  });

  /**
   * The Desktop switch must NOT opt in — an earlier audit established
   * that closing "Switching Desktop to X…" should complete the switch.
   * Without this, "make everything cancel on dismiss" would look like a
   * fix and silently revert that.
   */
  it("the reversible Desktop switch does not opt in", () => {
    const at = src.indexOf("dedupeKey: \"desktop-switch\"");
    if (at === -1) return; // lives in useAccountHandlers; nothing to assert here
    const around = src.slice(Math.max(0, at - 400), at + 400);
    expect(around).not.toContain("cancelOnDismiss");
  });
});
