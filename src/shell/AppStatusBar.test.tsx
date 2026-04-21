import { describe, expect, it } from "vitest";
import { formatLiveSegment, modelMix } from "./AppStatusBar";
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
      const segment = formatLiveSegment([mkSession({ model: null })]);
      expect(segment).toBe("● 1 live · ? 1");
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
