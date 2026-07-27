// Tests for the Unused-view fetch wrapper.
//
// The substantive rules — identity, dedup across cached plugin
// versions, ledger subtraction, grace window, disabled-plugin exclusion
// — now live in `claudepot_core::artifact_usage::unused` and are tested
// there (plus the shared vectors in
// `artifactKey.vectors.test.ts`). What remains testable here is the
// fetch contract: gating, error handling, and pass-through.

import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ artifactUsageUnused: vi.fn() }));

vi.mock("../../api", () => ({
  api: { artifactUsageUnused: h.artifactUsageUnused },
}));

import { useUnusedArtifacts } from "./useUnusedArtifacts";

function report(over: Partial<Record<string, unknown>> = {}) {
  return {
    rows: [
      {
        kind: "skill",
        artifact_key: "userSettings:lonely",
        plugin_id: null,
        label: "lonely",
        abs_path: "/h/.claude/skills/lonely/SKILL.md",
        modified_ms: 1_700_000_000_000,
      },
    ],
    installed_count: 10,
    suppressed_recent: 2,
    suppressed_disabled: 3,
    grace_days: 7,
    ...over,
  };
}

describe("useUnusedArtifacts", () => {
  it("does not fetch until the view is enabled", () => {
    renderHook(() => useUnusedArtifacts(false));
    expect(h.artifactUsageUnused).not.toHaveBeenCalled();
  });

  it("passes the core report through without re-deriving anything", async () => {
    h.artifactUsageUnused.mockResolvedValue(report());
    const { result } = renderHook(() => useUnusedArtifacts(true));
    await waitFor(() => expect(result.current.rows).toHaveLength(1));

    expect(result.current.rows[0].artifact_key).toBe("userSettings:lonely");
    expect(result.current.installedCount).toBe(10);
    expect(result.current.suppressedRecent).toBe(2);
    expect(result.current.suppressedDisabled).toBe(3);
    // Read from the payload, not a duplicated frontend constant — the
    // UI must state the window core actually applied.
    expect(result.current.graceDays).toBe(7);
  });

  it("surfaces a failure instead of reporting an empty unused set", async () => {
    // Failing open would render "nothing is unused", which is a
    // confident wrong answer rather than an honest error.
    h.artifactUsageUnused.mockRejectedValue(new Error("db locked"));
    const { result } = renderHook(() => useUnusedArtifacts(true));

    // Wait on the signal under test, not on `loading` — that starts
    // false, so waiting for it would resolve before the fetch settles
    // and let the rejection land after teardown.
    await waitFor(() => expect(result.current.error).toBeTruthy());

    expect(result.current.error).toContain("db locked");
    expect(result.current.rows).toHaveLength(0);
  });

  it("does not refetch on a plain re-render", async () => {
    // Self-isolating: measures the delta rather than depending on a
    // shared beforeEach reset. A global mock reset here breaks the
    // error-path test above by detaching its rejection handler.
    h.artifactUsageUnused.mockResolvedValue(report());
    const before = h.artifactUsageUnused.mock.calls.length;
    const { result, rerender } = renderHook(() => useUnusedArtifacts(true));
    await waitFor(() => expect(result.current.rows).toHaveLength(1));
    const afterMount = h.artifactUsageUnused.mock.calls.length;
    rerender();
    expect(h.artifactUsageUnused.mock.calls.length).toBe(afterMount);
    expect(afterMount).toBeGreaterThan(before);
  });
});
