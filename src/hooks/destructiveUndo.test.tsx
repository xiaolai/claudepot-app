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
