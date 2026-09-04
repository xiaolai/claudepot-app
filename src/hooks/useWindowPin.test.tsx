import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { useWindowPin } from "./useWindowPin";
import type { Preferences } from "../types";

vi.mock("../api", () => ({
  api: {
    preferencesGet: vi.fn(),
    preferencesSetWindowAlwaysOnTop: vi.fn(),
  },
}));

/** Only the field the hook reads; the rest of the record is irrelevant
 *  to it, and a full literal here would have to track every future
 *  preference. */
function prefs(windowAlwaysOnTop: boolean): Preferences {
  return { window_always_on_top: windowAlwaysOnTop } as unknown as Preferences;
}

const get = vi.mocked(api.preferencesGet);
const set = vi.mocked(api.preferencesSetWindowAlwaysOnTop);

/** The `cp-prefs-changed` handler the hook registered, via the global
 *  `listen` mock in test/setup.ts. */
function prefsChangedHandler(): (ev: { payload: Preferences }) => void {
  const call = vi
    .mocked(listen)
    .mock.calls.find(([channel]) => channel === "cp-prefs-changed");
  if (!call) throw new Error("hook did not subscribe to cp-prefs-changed");
  return call[1] as (ev: { payload: Preferences }) => void;
}

describe("useWindowPin", () => {
  beforeEach(() => {
    get.mockReset();
    set.mockReset();
    vi.mocked(listen).mockClear();
    get.mockResolvedValue(prefs(false));
  });

  it("starts unpinned and then reads the persisted value", async () => {
    get.mockResolvedValue(prefs(true));
    const { result } = renderHook(() => useWindowPin(() => {}));
    expect(result.current.pinned).toBe(false);
    await waitFor(() => expect(result.current.pinned).toBe(true));
  });

  it("toggle is optimistic and settles on the backend's answer", async () => {
    set.mockResolvedValue(prefs(true));
    const { result } = renderHook(() => useWindowPin(() => {}));
    await waitFor(() => expect(get).toHaveBeenCalled());
    act(() => result.current.toggle());
    expect(result.current.pinned).toBe(true);
    expect(set).toHaveBeenCalledWith(true);
    await waitFor(() => expect(result.current.pinned).toBe(true));
  });

  it("a failed toggle reverts the button and reports the error", async () => {
    const boom = new Error("save failed");
    set.mockRejectedValue(boom);
    const onError = vi.fn();
    const { result } = renderHook(() => useWindowPin(onError));
    await waitFor(() => expect(get).toHaveBeenCalled());
    act(() => result.current.toggle());
    expect(result.current.pinned).toBe(true);
    await waitFor(() => expect(result.current.pinned).toBe(false));
    expect(onError).toHaveBeenCalledWith(boom);
  });

  it("follows cp-prefs-changed from elsewhere", async () => {
    const { result } = renderHook(() => useWindowPin(() => {}));
    await waitFor(() => expect(get).toHaveBeenCalled());
    act(() => prefsChangedHandler()({ payload: prefs(true) }));
    expect(result.current.pinned).toBe(true);
    act(() => prefsChangedHandler()({ payload: prefs(false) }));
    expect(result.current.pinned).toBe(false);
  });
});
