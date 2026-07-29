import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

const { projectList } = vi.hoisted(() => ({ projectList: vi.fn() }));
vi.mock("../api", () => ({ api: { projectList } }));

import { __resetProjectCache, useProjectSearch } from "./useProjectSearch";
import type { ProjectInfo } from "../types";

function project(path: string): ProjectInfo {
  return {
    sanitized_name: path.replace(/[/\\]/g, "-"),
    original_path: path,
    session_count: 3,
    memory_file_count: 0,
    total_size_bytes: 1,
    last_modified_ms: 1,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  __resetProjectCache();
  projectList.mockResolvedValue([]);
});

describe("useProjectSearch", () => {
  it("does not fetch below the search threshold", () => {
    renderHook(() => useProjectSearch("a"));
    expect(projectList).not.toHaveBeenCalled();
  });

  it("fetches once and matches on basename", async () => {
    projectList.mockResolvedValue([
      project("/Users/j/github/claudepot"),
      project("/Users/j/work/other"),
    ]);
    const { result } = renderHook(() => useProjectSearch("claudepot"));

    await waitFor(() => expect(result.current.hits).toHaveLength(1));
    expect(result.current.hits[0]!.original_path).toBe(
      "/Users/j/github/claudepot",
    );
  });

  it("matches a Windows path on its basename", async () => {
    projectList.mockResolvedValue([project("C:\\Users\\j\\code\\widget")]);
    const { result } = renderHook(() => useProjectSearch("widget"));
    await waitFor(() => expect(result.current.hits).toHaveLength(1));
  });

  it("shares one request across concurrent hooks", async () => {
    projectList.mockResolvedValue([project("/a/beta")]);
    const a = renderHook(() => useProjectSearch("beta"));
    const b = renderHook(() => useProjectSearch("beta"));
    await waitFor(() => expect(a.result.current.hits).toHaveLength(1));
    await waitFor(() => expect(b.result.current.hits).toHaveLength(1));
    expect(projectList).toHaveBeenCalledTimes(1);
  });

  it("never gets stuck loading when a request is disowned mid-flight", async () => {
    // The generation guard drops the result of an invalidated request.
    // Dropping it without clearing `loading` (and without a retry)
    // left the hook reporting "…searching" forever.
    let resolveFirst: (v: ProjectInfo[]) => void = () => {};
    projectList.mockImplementationOnce(
      () => new Promise<ProjectInfo[]>((res) => { resolveFirst = res; }),
    );
    const { result } = renderHook(() => useProjectSearch("thing"));
    await waitFor(() => expect(result.current.loading).toBe(true));

    // Invalidate while in flight, then let the stale request land.
    __resetProjectCache();
    projectList.mockResolvedValue([project("/x/thing")]);
    resolveFirst([project("/stale/thing")]);

    await waitFor(() => expect(result.current.loading).toBe(false));
    // It recovers with the fresh data rather than the disowned result.
    await waitFor(() =>
      expect(result.current.hits[0]?.original_path).toBe("/x/thing"),
    );
  });

  it("degrades to no hits when the backend errors", async () => {
    projectList.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useProjectSearch("anything"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.hits).toEqual([]);
  });
});
