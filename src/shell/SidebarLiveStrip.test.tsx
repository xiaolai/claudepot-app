import { describe, expect, it } from "vitest";
import {
  formatElapsed,
  projectLabel,
  shortenModel,
  sortForStrip,
} from "./SidebarLiveStrip";
import type { LiveSessionSummary } from "../types";

function session(over: Partial<LiveSessionSummary> = {}): LiveSessionSummary {
  return {
    session_id: "s",
    pid: 1,
    cwd: "/tmp/x",
    transcript_path: null,
    status: "idle",
    current_action: null,
    model: null,
    waiting_for: null,
    errored: false,
    stuck: false,
    idle_ms: 0,
    seq: 0,
    ...over,
  };
}

describe("SidebarLiveStrip helpers", () => {
  describe("projectLabel", () => {
    it("returns the last path segment", () => {
      expect(projectLabel("/Users/joker/projects/claudepot")).toBe("claudepot");
    });

    it("strips trailing slashes before splitting", () => {
      expect(projectLabel("/tmp/project/")).toBe("project");
    });

    it("returns the cwd verbatim when there is no slash", () => {
      expect(projectLabel("rootless")).toBe("rootless");
    });

    it("tolerates an empty-after-trim path", () => {
      expect(projectLabel("/")).toBe("/");
    });
  });

  describe("formatElapsed", () => {
    it("renders a dash for sub-second durations", () => {
      expect(formatElapsed(0)).toBe("—");
      expect(formatElapsed(999)).toBe("—");
    });

    it("uses Ns form for under 10 seconds", () => {
      expect(formatElapsed(1000)).toBe("1s");
      expect(formatElapsed(9_999)).toBe("9s");
    });

    it("uses M:SS form for minutes under an hour", () => {
      expect(formatElapsed(61_000)).toBe("1:01");
      expect(formatElapsed(754_000)).toBe("12:34");
    });

    it("uses HhMm form for hour-scale durations", () => {
      expect(formatElapsed(3_600_000)).toBe("1h0m");
      expect(formatElapsed(3_600_000 + 17 * 60_000)).toBe("1h17m");
    });
  });

  describe("shortenModel", () => {
    it("collapses dated variants to family 3-letter markers", () => {
      expect(shortenModel("claude-opus-4-7")).toBe("OPUS");
      expect(shortenModel("claude-sonnet-4-6")).toBe("SON");
      expect(shortenModel("claude-haiku-4-5-20251001")).toBe("HAI");
    });

    it("passes short unknowns through", () => {
      expect(shortenModel("custom")).toBe("custom");
    });

    it("ellipsifies long unknowns", () => {
      expect(shortenModel("some-unusual-provider")).toBe("some-unu…");
    });

    it("returns empty for null", () => {
      expect(shortenModel(null)).toBe("");
    });
  });

  describe("sortForStrip", () => {
    it("orders alerting → busy → waiting → idle", () => {
      const input = [
        session({ session_id: "idle", status: "idle", idle_ms: 0 }),
        session({ session_id: "wait", status: "waiting", idle_ms: 0 }),
        session({ session_id: "busy", status: "busy", idle_ms: 0 }),
        session({
          session_id: "alert",
          status: "busy",
          idle_ms: 0,
          errored: true,
        }),
      ];
      expect(sortForStrip(input).map((s) => s.session_id)).toEqual([
        "alert",
        "busy",
        "wait",
        "idle",
      ]);
    });

    it("breaks ties by ascending idle_ms within a tier", () => {
      const input = [
        session({ session_id: "old", status: "idle", idle_ms: 5_000 }),
        session({ session_id: "new", status: "idle", idle_ms: 1_000 }),
      ];
      expect(sortForStrip(input).map((s) => s.session_id)).toEqual([
        "new",
        "old",
      ]);
    });

    it("does not mutate its input", () => {
      const input = [
        session({ session_id: "a", status: "idle", idle_ms: 9 }),
        session({ session_id: "b", status: "busy", idle_ms: 0 }),
      ];
      const snapshot = input.map((s) => s.session_id);
      sortForStrip(input);
      expect(input.map((s) => s.session_id)).toEqual(snapshot);
    });
  });
});
