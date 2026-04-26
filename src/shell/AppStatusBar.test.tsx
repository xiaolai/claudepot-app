import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { AppStatusBar, formatLiveSegment, modelMix } from "./AppStatusBar";
import type { LiveSessionSummary } from "../types";

function mkSession(overrides: Partial<LiveSessionSummary> = {}): LiveSessionSummary {
  return {
    session_id: overrides.session_id ?? "s",
    pid: overrides.pid ?? 1,
    cwd: overrides.cwd ?? "/tmp/p",
    transcript_path: null,
    status: overrides.status ?? "busy",
    current_action: null,
    model: overrides.model ?? null,
    waiting_for: null,
    errored: false,
    stuck: false,
    idle_ms: 0,
    seq: 0,
  };
}

describe("AppStatusBar helpers", () => {
  describe("formatLiveSegment", () => {
    it("returns null when no sessions are live", () => {
      expect(formatLiveSegment([])).toBeNull();
    });

    it("drops the model-mix tail when every session has unknown model", () => {
      // Unknown-model sessions are counted in the live segment but are
      // not surfaced in the mix — a lone "?" letterform reads as an
      // error indicator at a glance.
      const segment = formatLiveSegment([mkSession({ model: null })]);
      expect(segment).toBe("● 1 live");
    });

    it("renders the mix even when some sessions are still unknown", () => {
      const segment = formatLiveSegment([
        mkSession({ model: null }),
        mkSession({ model: null }),
        mkSession({ model: "claude-opus-4-7" }),
      ]);
      // Three live total, only one known family → no "? 2" tail.
      expect(segment).toBe("● 3 live · OPUS 1");
    });

    it("renders counts with family markers", () => {
      const sessions = [
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-sonnet-4-6" }),
      ];
      expect(formatLiveSegment(sessions)).toBe("● 3 live · OPUS 2, SON 1");
    });
  });

  describe("modelMix", () => {
    it("groups by family", () => {
      const sessions = [
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-opus-4-7-20251001" }),
        mkSession({ model: "claude-sonnet-4-6" }),
        mkSession({ model: "claude-haiku-4-5" }),
      ];
      expect(modelMix(sessions)).toEqual(["OPUS 2", "HAI 1", "SON 1"]);
    });

    it("sorts by count desc then key asc", () => {
      const sessions = [
        mkSession({ model: "claude-sonnet-4-6" }),
        mkSession({ model: "claude-haiku-4-5" }),
      ];
      // Ties break alphabetically: HAI before SON.
      expect(modelMix(sessions)).toEqual(["HAI 1", "SON 1"]);
    });

    it("truncates unmapped long models", () => {
      const sessions = [mkSession({ model: "some-very-long-id" })];
      expect(modelMix(sessions)[0]).toBe("some-ve… 1");
    });
  });
});

/**
 * Component-level coverage for the dismissed-toast echo segment. The
 * unit tests in `useToasts.test.ts` lock down the hook contract; this
 * suite locks down what the bar actually renders given that contract:
 * suppression while a live toast is on screen, render-after-dismiss,
 * and auto-clear after the fade window.
 *
 * `vi.mock` is hoisted by vitest to before the imports, so its
 * factory cannot close over a locally-declared `state` variable. We
 * route the per-test state through `vi.hoisted` so both the mock
 * factory and the test bodies share the same object.
 */
const echoMocks = vi.hoisted(() => {
  type ToastShape = { id: number; kind: "info" | "error"; text: string };
  type Dismissed =
    | { text: string; kind: "info" | "error"; at: number }
    | null;
  const state: {
    toasts: ToastShape[];
    lastDismissed: Dismissed;
    clearLastDismissedSpy: ReturnType<typeof vi.fn> | null;
  } = { toasts: [], lastDismissed: null, clearLastDismissedSpy: null };
  return { state };
});

vi.mock("../hooks/useSessionLive", () => ({
  useSessionLive: () => [],
}));
vi.mock("../providers/AppStateProvider", () => ({
  useAppState: () => ({
    toasts: echoMocks.state.toasts,
    lastDismissed: echoMocks.state.lastDismissed,
    clearLastDismissed:
      echoMocks.state.clearLastDismissedSpy ?? (() => undefined),
  }),
}));

describe("AppStatusBar — toast echo", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    echoMocks.state.toasts = [];
    echoMocks.state.lastDismissed = null;
    echoMocks.state.clearLastDismissedSpy = vi.fn();
  });
  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  const stats = { projects: null, sessions: null };

  it("does not render the echo when nothing has been dismissed yet", () => {
    render(<AppStatusBar stats={stats} />);
    // The echo is the only descendant tagged aria-hidden + ellipsised.
    expect(document.querySelector('[aria-hidden="true"]')).toBeNull();
  });

  it("suppresses the echo while a live toast is on screen", () => {
    echoMocks.state.lastDismissed = {
      text: "Copied path.",
      kind: "info",
      at: Date.now(),
    };
    echoMocks.state.toasts = [{ id: 1, kind: "info", text: "live" }];
    render(<AppStatusBar stats={stats} />);
    // Echo would otherwise read "Copied path." — suppression means
    // the text never reaches the DOM at all.
    expect(screen.queryByText("Copied path.")).toBeNull();
  });

  it("renders the echo text after a dismissal with no live toast", () => {
    echoMocks.state.lastDismissed = {
      text: "Rename complete.",
      kind: "info",
      at: Date.now(),
    };
    render(<AppStatusBar stats={stats} />);
    expect(screen.getByText("Rename complete.")).toBeInTheDocument();
  });

  it("calls clearLastDismissed once the 6 s window elapses", () => {
    echoMocks.state.lastDismissed = {
      text: "Saved.",
      kind: "info",
      at: Date.now(),
    };
    render(<AppStatusBar stats={stats} />);
    expect(echoMocks.state.clearLastDismissedSpy).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(6_000));
    expect(echoMocks.state.clearLastDismissedSpy).toHaveBeenCalledTimes(1);
  });

  it("clears immediately if the dismissal was already older than the window", () => {
    // Edge case: dismissal happened, then the user navigated away from
    // the section. When the bar mounts, the echo is already past its
    // sell-by date — clear synchronously rather than schedule a timer
    // that would fire instantly.
    echoMocks.state.lastDismissed = {
      text: "stale",
      kind: "info",
      at: Date.now() - 10_000,
    };
    render(<AppStatusBar stats={stats} />);
    expect(echoMocks.state.clearLastDismissedSpy).toHaveBeenCalledTimes(1);
  });
});
