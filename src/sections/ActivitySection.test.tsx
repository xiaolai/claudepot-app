import { describe, expect, it } from "vitest";
import {
  aggregateModelMix,
  countByStatus,
  describeAction,
  familyShort,
  formatElapsedMs,
  projectLabel,
  sortSessions,
  statusBreakdown,
} from "./ActivitySection";
import type { LiveSessionSummary } from "../types";

function mkSession(
  overrides: Partial<LiveSessionSummary> = {},
): LiveSessionSummary {
  return {
    session_id: overrides.session_id ?? "s",
    pid: overrides.pid ?? 1,
    cwd: overrides.cwd ?? "/tmp/p",
    transcript_path: null,
    status: overrides.status ?? "busy",
    current_action: overrides.current_action ?? null,
    model: overrides.model ?? null,
    waiting_for: overrides.waiting_for ?? null,
    errored: overrides.errored ?? false,
    stuck: overrides.stuck ?? false,
    idle_ms: overrides.idle_ms ?? 0,
    seq: overrides.seq ?? 0,
  };
}

describe("ActivitySection helpers", () => {
  describe("projectLabel", () => {
    it("returns last path segment", () => {
      expect(projectLabel("/tmp/project/claudepot")).toBe("claudepot");
    });
    it("strips trailing slashes", () => {
      expect(projectLabel("/tmp/project/")).toBe("project");
    });
    it("falls back to full path for root or empty-segment", () => {
      expect(projectLabel("/")).toBe("/");
    });
    it("handles Windows-style backslash paths", () => {
      expect(projectLabel("C:\\work\\myproject")).toBe("myproject");
      expect(projectLabel("C:\\work\\myproject\\")).toBe("myproject");
    });
  });

  describe("familyShort", () => {
    it("maps known families", () => {
      expect(familyShort("claude-opus-4-7")).toBe("OPUS");
      expect(familyShort("claude-sonnet-4-6")).toBe("SON");
      expect(familyShort("claude-haiku-4-5-20251001")).toBe("HAI");
    });
    it("returns em dash for null", () => {
      expect(familyShort(null)).toBe("—");
    });
    it("passes short unknowns through", () => {
      expect(familyShort("custom")).toBe("custom");
    });
    it("ellipsifies long unknowns", () => {
      expect(familyShort("some-provider-name")).toBe("some-prov…");
    });
  });

  describe("describeAction", () => {
    it("prefers current_action", () => {
      expect(
        describeAction(mkSession({ current_action: "Bash: pnpm test" })),
      ).toBe("Bash: pnpm test");
    });
    it("uses waiting verb when waiting", () => {
      expect(
        describeAction(
          mkSession({ status: "waiting", waiting_for: "approve Bash" }),
        ),
      ).toBe("waiting — approve Bash");
    });
    it("has microcopy for idle", () => {
      expect(describeAction(mkSession({ status: "idle" }))).toBe(
        "idle — awaiting prompt",
      );
    });
    it("falls back to working… for busy sans action", () => {
      expect(describeAction(mkSession({ status: "busy" }))).toBe(
        "working…",
      );
    });
  });

  describe("formatElapsedMs", () => {
    it("sub-second shows dash", () => {
      expect(formatElapsedMs(500)).toBe("—");
    });
    it("under 10s shows Ns", () => {
      expect(formatElapsedMs(5_000)).toBe("5s");
    });
    it("minutes show M:SS", () => {
      expect(formatElapsedMs(70_000)).toBe("1:10");
    });
    it("hours show HhMm", () => {
      expect(formatElapsedMs(3_600_000 + 30 * 60_000)).toBe("1h30m");
    });
  });

  describe("countByStatus", () => {
    it("counts each status independently", () => {
      const sessions = [
        mkSession({ status: "busy" }),
        mkSession({ status: "busy" }),
        mkSession({ status: "waiting" }),
        mkSession({ status: "idle" }),
      ];
      expect(countByStatus(sessions, "busy")).toBe(2);
      expect(countByStatus(sessions, "waiting")).toBe(1);
      expect(countByStatus(sessions, "idle")).toBe(1);
    });
  });

  describe("statusBreakdown (render-if-nonzero)", () => {
    it("drops zero-count segments", () => {
      // Three waiting, zero busy, zero idle → no 'busy'/'idle'
      // terms in the joined string (design.md rule).
      const sessions = [
        mkSession({ status: "waiting" }),
        mkSession({ status: "waiting" }),
        mkSession({ status: "waiting" }),
      ];
      expect(statusBreakdown(sessions)).toBe("3 waiting");
    });

    it("joins present counts in busy/waiting/idle order", () => {
      const sessions = [
        mkSession({ status: "busy" }),
        mkSession({ status: "busy" }),
        mkSession({ status: "idle" }),
      ];
      expect(statusBreakdown(sessions)).toBe("2 busy · 1 idle");
    });

    it("falls back to em dash when every status is zero", () => {
      // Defensive — parents gate on length > 0, but the helper
      // should not produce a stray "" string.
      expect(statusBreakdown([])).toBe("—");
    });
  });

  describe("aggregateModelMix", () => {
    it("groups and orders desc count then asc key", () => {
      const sessions = [
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-opus-4-7" }),
        mkSession({ model: "claude-sonnet-4-6" }),
        mkSession({ model: "claude-haiku-4-5" }),
      ];
      expect(aggregateModelMix(sessions)).toEqual([
        "OPUS 2",
        "HAI 1",
        "SON 1",
      ]);
    });
  });

  describe("sortSessions", () => {
    it("alerting > busy > waiting > idle", () => {
      const sessions = [
        mkSession({ session_id: "idle", status: "idle", idle_ms: 1000 }),
        mkSession({ session_id: "busy", status: "busy", idle_ms: 500 }),
        mkSession({ session_id: "waiting", status: "waiting", idle_ms: 200 }),
        mkSession({ session_id: "errored", status: "busy", errored: true, idle_ms: 2000 }),
        mkSession({ session_id: "stuck", status: "busy", stuck: true, idle_ms: 3000 }),
      ];
      const ids = sortSessions(sessions).map((s) => s.session_id);
      expect(ids.indexOf("errored")).toBeLessThan(ids.indexOf("busy"));
      expect(ids.indexOf("stuck")).toBeLessThan(ids.indexOf("busy"));
      expect(ids.indexOf("busy")).toBeLessThan(ids.indexOf("waiting"));
      expect(ids.indexOf("waiting")).toBeLessThan(ids.indexOf("idle"));
    });

    it("within a tier, ascending idle_ms (most recently active first)", () => {
      const sessions = [
        mkSession({ session_id: "a", status: "busy", idle_ms: 5000 }),
        mkSession({ session_id: "b", status: "busy", idle_ms: 100 }),
        mkSession({ session_id: "c", status: "busy", idle_ms: 2000 }),
      ];
      const ids = sortSessions(sessions).map((s) => s.session_id);
      expect(ids).toEqual(["b", "c", "a"]);
    });

    it("does not mutate the input array", () => {
      const sessions = [
        mkSession({ session_id: "z", status: "idle", idle_ms: 1 }),
        mkSession({ session_id: "a", status: "busy", idle_ms: 1 }),
      ];
      const original = [...sessions];
      sortSessions(sessions);
      expect(sessions.map((s) => s.session_id)).toEqual(
        original.map((s) => s.session_id),
      );
    });

    it("errored sessions in idle tier still sort to the top", () => {
      const sessions = [
        mkSession({ session_id: "idle-ok", status: "idle", idle_ms: 10 }),
        mkSession({ session_id: "idle-err", status: "idle", errored: true, idle_ms: 9999 }),
      ];
      const ids = sortSessions(sessions).map((s) => s.session_id);
      expect(ids[0]).toBe("idle-err");
    });
  });
});
