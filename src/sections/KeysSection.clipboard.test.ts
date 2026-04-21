import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  CLIPBOARD_CLEAR_MS,
  scheduleClipboardClear,
} from "./KeysSection";

describe("scheduleClipboardClear", () => {
  let readText: ReturnType<typeof vi.fn>;
  let writeText: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers();
    readText = vi.fn();
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText, writeText },
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("clears the clipboard when it still holds the token", async () => {
    readText.mockResolvedValue("sk-ant-secret-xyz");
    scheduleClipboardClear("sk-ant-secret-xyz");

    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS);
    await Promise.resolve();
    await Promise.resolve();

    expect(readText).toHaveBeenCalledOnce();
    expect(writeText).toHaveBeenCalledWith("");
  });

  it("does NOT clear when the clipboard changed to something else", async () => {
    readText.mockResolvedValue("user copied something else");
    scheduleClipboardClear("sk-ant-secret-xyz");

    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS);
    await Promise.resolve();

    expect(readText).toHaveBeenCalledOnce();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("aborts instead of blind-clearing when readText is denied", async () => {
    readText.mockRejectedValue(new Error("NotAllowedError"));
    scheduleClipboardClear("sk-ant-secret-xyz");

    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS);
    await Promise.resolve();
    await Promise.resolve();

    expect(readText).toHaveBeenCalledOnce();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("does not fire before the 30s delay", async () => {
    readText.mockResolvedValue("sk-ant-secret-xyz");
    scheduleClipboardClear("sk-ant-secret-xyz");

    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS - 1);

    expect(readText).not.toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
  });
});
