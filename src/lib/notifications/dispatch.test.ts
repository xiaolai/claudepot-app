import { describe, it, expect, vi } from "vitest";
import { buildEmit } from "./dispatch";
import type {
  EmitDeps,
  LogAppendRoutedFn,
  LogMarkDeliveredFn,
  OsDispatchFn,
  ToastPushFn,
} from "./dispatch";

// Tests focus on the load-bearing invariants of the dispatcher:
//   1. Exactly ONE log entry per logical event.
//   2. Surfaces fan out exactly per `route()`.
//   3. OS delivery outcome is reported back via mark_delivered.
//   4. Toast action fields wire through to the primitive.
//   5. Category overrides (rotationApplied auto-mode, rotationSuggested
//      forces toast) actually influence dispatch.

function makeDeps(overrides?: {
  pushToast?: ToastPushFn;
  dispatchOs?: OsDispatchFn;
  logAppend?: LogAppendRoutedFn;
  logMark?: LogMarkDeliveredFn;
  windowFocused?: boolean;
  rotationMode?: "confirm" | "auto";
}) {
  const pushToast: ToastPushFn = overrides?.pushToast ?? vi.fn();
  const dispatchOs: OsDispatchFn =
    overrides?.dispatchOs ?? (vi.fn(async () => true) as OsDispatchFn);
  const logAppendRouted: LogAppendRoutedFn =
    overrides?.logAppend ?? (vi.fn(async () => 42) as LogAppendRoutedFn);
  const logMarkDelivered: LogMarkDeliveredFn =
    overrides?.logMark ?? (vi.fn(async () => true) as LogMarkDeliveredFn);
  const deps: EmitDeps = {
    pushToast,
    dispatchOs,
    logAppendRouted,
    logMarkDelivered,
    getContext: () => ({
      windowFocused: overrides?.windowFocused ?? false,
      rotationMode: overrides?.rotationMode,
    }),
    // Tests default to "enabled + priority default OS" so the
    // existing routing assertions hold without referencing the
    // global prefs cache. Category-pref behavior gets its own
    // suite below with explicit overrides.
    getPref: () => ({ enabled: true, osOverride: null }),
  };
  return {
    deps,
    pushToast: pushToast as unknown as ReturnType<typeof vi.fn>,
    dispatchOs: dispatchOs as unknown as ReturnType<typeof vi.fn>,
    logAppendRouted: logAppendRouted as unknown as ReturnType<typeof vi.fn>,
    logMarkDelivered: logMarkDelivered as unknown as ReturnType<typeof vi.fn>,
  };
}

describe("emit() single-row logging invariant", () => {
  it("writes exactly ONE log entry per P2 (toast-only) event", async () => {
    const { deps, logAppendRouted, pushToast } = makeDeps();
    const emit = buildEmit(deps);
    const result = await emit({
      category: "projectRenamed",
      title: "Renamed",
      body: "old → new",
    });
    expect(logAppendRouted).toHaveBeenCalledTimes(1);
    expect(pushToast).toHaveBeenCalledTimes(1);
    expect(result.logId).toBe(42);
    // surfaces_requested = [toast]; toast is renderer-side so
    // surfaces_delivered = [toast] is set BEFORE the OS dispatcher
    // runs (it doesn't, for P2 — but the invariant is "log records
    // intent + known-delivered, with mark_delivered for OS later").
    expect(logAppendRouted).toHaveBeenCalledWith(
      expect.objectContaining({
        category: "projectRenamed",
        priority: "p2Acknowledge",
        surfacesRequested: ["toast"],
        surfacesDelivered: ["toast"],
      }),
    );
  });

  it("writes exactly ONE log entry per P1 (os-only) event", async () => {
    const { deps, logAppendRouted, dispatchOs, pushToast } = makeDeps();
    const emit = buildEmit(deps);
    await emit({
      category: "usageThreshold",
      title: "Near cap",
    });
    expect(logAppendRouted).toHaveBeenCalledTimes(1);
    expect(pushToast).not.toHaveBeenCalled();
    expect(dispatchOs).toHaveBeenCalledTimes(1);
  });

  it("writes exactly ONE log entry per RotationSuggested event (toast + OS)", async () => {
    // The bug the audit flagged: rotation events used to log TWICE
    // (once from pushToast, once from dispatchOsNotification). The
    // emit() facade fixes this by calling primitives with
    // `_suppressLog: true` and writing one routed log entry.
    const { deps, logAppendRouted, pushToast, dispatchOs } = makeDeps();
    const emit = buildEmit(deps);
    await emit({
      category: "rotationSuggested",
      title: "Auto-rotation suggested",
      body: "a@b → c@d",
      toastAction: { label: "Switch", onPress: vi.fn() },
    });
    expect(logAppendRouted).toHaveBeenCalledTimes(1);
    expect(pushToast).toHaveBeenCalledTimes(1);
    expect(dispatchOs).toHaveBeenCalledTimes(1);
  });
});

describe("primitive shim — _suppressLog", () => {
  it("every primitive call carries _suppressLog: true", async () => {
    const { deps, pushToast, dispatchOs } = makeDeps();
    const emit = buildEmit(deps);
    await emit({
      category: "rotationSuggested",
      title: "x",
      toastAction: { label: "Go", onPress: vi.fn() },
    });
    // pushToast: 4th arg is opts; _suppressLog must be true
    expect(pushToast).toHaveBeenCalledWith(
      "info",
      "x",
      expect.any(Function),
      expect.objectContaining({ _suppressLog: true }),
    );
    // dispatchOs: 3rd arg is opts
    expect(dispatchOs).toHaveBeenCalledWith(
      "x",
      "",
      expect.objectContaining({ _suppressLog: true }),
    );
  });
});

describe("OS delivery mark-back", () => {
  it("marks OS surface delivered when dispatchOs returns true", async () => {
    const { deps, logMarkDelivered } = makeDeps({
      dispatchOs: vi.fn(async () => true) as OsDispatchFn,
    });
    const emit = buildEmit(deps);
    await emit({
      category: "usageThreshold",
      title: "x",
    });
    expect(logMarkDelivered).toHaveBeenCalledWith(42, "osBanner");
  });

  it("does NOT mark OS surface delivered when dispatchOs returns false", async () => {
    const { deps, logMarkDelivered } = makeDeps({
      dispatchOs: vi.fn(async () => false) as OsDispatchFn,
    });
    const emit = buildEmit(deps);
    const result = await emit({
      category: "usageThreshold",
      title: "x",
    });
    expect(logMarkDelivered).not.toHaveBeenCalled();
    expect(result.delivered).not.toContain("osBanner");
  });
});

describe("toast action wiring", () => {
  it("forwards onPress to pushToast's onUndo slot", async () => {
    const onPress = vi.fn();
    const onCommit = vi.fn();
    const { deps, pushToast } = makeDeps();
    const emit = buildEmit(deps);
    await emit({
      category: "rotationSuggested",
      title: "x",
      toastAction: { label: "Switch", onPress, onCommit, timeoutMs: 25_000 },
    });
    expect(pushToast).toHaveBeenCalledWith(
      "info",
      "x",
      onPress,
      expect.objectContaining({
        undoLabel: "Switch",
        undoMs: 25_000,
        onCommit,
      }),
    );
  });
});

describe("category overrides via DispatchContext", () => {
  it("RotationApplied is silent in auto mode (no toast)", async () => {
    const { deps, pushToast, logAppendRouted } = makeDeps({
      rotationMode: "auto",
    });
    const emit = buildEmit(deps);
    await emit({
      category: "rotationApplied",
      title: "Applied",
    });
    expect(pushToast).not.toHaveBeenCalled();
    // But the log entry STILL lands — silent rotation logs.
    expect(logAppendRouted).toHaveBeenCalledTimes(1);
    expect(logAppendRouted).toHaveBeenCalledWith(
      expect.objectContaining({
        surfacesRequested: [],
        surfacesDelivered: [],
      }),
    );
  });

  it("RotationApplied shows toast in confirm mode", async () => {
    const { deps, pushToast } = makeDeps({ rotationMode: "confirm" });
    const emit = buildEmit(deps);
    await emit({
      category: "rotationApplied",
      title: "Applied",
    });
    expect(pushToast).toHaveBeenCalledTimes(1);
  });
});

describe("CategoryPrefs gating", () => {
  it("enabled=false produces a log-only row (no toast, no OS)", async () => {
    const { deps, pushToast, dispatchOs, logAppendRouted } = makeDeps();
    deps.getPref = () => ({ enabled: false, osOverride: null });
    const emit = buildEmit(deps);
    await emit({ category: "projectRenamed", title: "Renamed" });
    expect(pushToast).not.toHaveBeenCalled();
    expect(dispatchOs).not.toHaveBeenCalled();
    // Log still lands — bell records the forensic trail of
    // suppressed notifications.
    expect(logAppendRouted).toHaveBeenCalledTimes(1);
    expect(logAppendRouted).toHaveBeenCalledWith(
      expect.objectContaining({
        surfacesRequested: [],
        surfacesDelivered: [],
      }),
    );
  });

  it("osOverride=false suppresses OS even when priority wants it", async () => {
    const { deps, dispatchOs, logAppendRouted } = makeDeps();
    deps.getPref = () => ({ enabled: true, osOverride: false });
    const emit = buildEmit(deps);
    await emit({ category: "usageThreshold", title: "Near cap" });
    expect(dispatchOs).not.toHaveBeenCalled();
    expect(logAppendRouted).toHaveBeenCalledWith(
      expect.objectContaining({
        surfacesRequested: [],
        surfacesDelivered: [],
      }),
    );
  });

  it("osOverride=true forces OS even on P2 (toast-only) categories", async () => {
    const { deps, dispatchOs, pushToast } = makeDeps();
    deps.getPref = () => ({ enabled: true, osOverride: true });
    const emit = buildEmit(deps);
    await emit({ category: "projectRenamed", title: "Renamed" });
    // P2 default surfaces toast; override flips OS on.
    expect(pushToast).toHaveBeenCalledTimes(1);
    expect(dispatchOs).toHaveBeenCalledTimes(1);
  });

  it("osOverride=null follows category priority default", async () => {
    const { deps, dispatchOs, pushToast } = makeDeps();
    deps.getPref = () => ({ enabled: true, osOverride: null });
    const emit = buildEmit(deps);
    // P1 default: os only
    await emit({ category: "usageThreshold", title: "Near cap" });
    expect(pushToast).not.toHaveBeenCalled();
    expect(dispatchOs).toHaveBeenCalledTimes(1);
  });
});

describe("log failure is non-blocking", () => {
  it("dispatch surfaces still fan out when log IPC fails", async () => {
    const logAppend = vi.fn(async () => {
      throw new Error("IPC down");
    }) as unknown as LogAppendRoutedFn;
    const { deps, pushToast } = makeDeps({ logAppend });
    const emit = buildEmit(deps);
    const result = await emit({
      category: "projectRenamed",
      title: "Renamed",
    });
    expect(pushToast).toHaveBeenCalled();
    expect(result.logId).toBeNull();
  });
});
