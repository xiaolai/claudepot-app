import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { ProjectInfo } from "../../types";
import type { LiveSessionSummary } from "../../types/activity";

// Controllable stand-in for the useSessionLive singleton store —
// lets tests push a new live snapshot and observe which rows
// re-render, without a Tauri event bridge.
const liveStore = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  let value: unknown[] = [];
  return {
    listeners,
    get: () => value,
    set(next: unknown[]) {
      value = next;
      listeners.forEach((l) => l());
    },
  };
});

vi.mock("../../hooks/useSessionLive", async () => {
  const { useSyncExternalStore } = await import("react");
  return {
    useSessionLive: () =>
      useSyncExternalStore(
        (l: () => void) => {
          liveStore.listeners.add(l);
          return () => liveStore.listeners.delete(l);
        },
        () => liveStore.get(),
      ),
  };
});

// Render-count probe: every ProjectRow renders exactly one Glyph
// (the folder icon) when neither hovered nor active, and the table
// header renders exactly one (the active sort-direction arrow). The
// memo tests below assert on deltas of this counter to prove that
// memoized rows skip re-rendering on live updates.
const glyphProbe = vi.hoisted(() => ({ count: 0 }));

vi.mock("../../components/primitives/Glyph", () => ({
  Glyph: () => {
    glyphProbe.count += 1;
    return <span data-testid="glyph" />;
  },
}));

import { ProjectsTable } from "./ProjectsTable";

function mk(
  path: string,
  mods: Partial<ProjectInfo> = {},
): ProjectInfo {
  return {
    sanitized_name: path.replace(/\//g, "-"),
    original_path: path,
    session_count: 0,
    memory_file_count: 0,
    total_size_bytes: 0,
    last_modified_ms: null,
    is_orphan: false,
    is_reachable: true,
    is_empty: false,
    ...mods,
  };
}

function pathsInOrder(): string[] {
  const list = screen.getByRole("listbox", { name: "Projects" });
  const items = within(list).getAllByRole("option");
  return items.map((li) => {
    // Each row has an inner span titled with the full original_path;
    // its text content is the basename we want.
    const titled = li.querySelector<HTMLSpanElement>("span[title]");
    return titled?.textContent ?? "";
  });
}

describe("ProjectsTable sort (G10)", () => {
  const fixtures = [
    mk("/alpha", { last_modified_ms: 3_000, total_size_bytes: 100, session_count: 3 }),
    mk("/beta", { last_modified_ms: 1_000, total_size_bytes: 500, session_count: 1 }),
    mk("/gamma", { last_modified_ms: 2_000, total_size_bytes: 50, session_count: 7 }),
  ];

  it("defaults to last_touched desc — freshest first", () => {
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    expect(pathsInOrder()).toEqual(["alpha", "gamma", "beta"]);
  });

  it("clicking Project column sorts asc by basename", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    await user.click(screen.getByRole("columnheader", { name: /Project/i }));
    expect(pathsInOrder()).toEqual(["alpha", "beta", "gamma"]);
  });

  it("clicking Size twice sorts desc — biggest first", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    const size = screen.getByRole("columnheader", { name: /Size/i });
    await user.click(size); // asc
    await user.click(size); // desc
    expect(pathsInOrder()).toEqual(["beta", "alpha", "gamma"]);
  });

  it("clicking the same column a third time restores default (last_touched desc)", async () => {
    const user = userEvent.setup();
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={() => {}}
      />,
    );
    const sessions = screen.getByRole("columnheader", { name: /Sessions/i });
    await user.click(sessions);
    await user.click(sessions);
    await user.click(sessions);
    expect(pathsInOrder()).toEqual(["alpha", "gamma", "beta"]);
  });
});

describe("ProjectsTable live updates (memoized rows)", () => {
  const fixtures = [
    mk("/alpha", { last_modified_ms: 3_000, session_count: 3 }),
    mk("/beta", { last_modified_ms: 1_000, session_count: 1 }),
    mk("/gamma", { last_modified_ms: 2_000, session_count: 7 }),
  ];
  const noopSelect = () => {};

  function mkLive(
    cwd: string,
    mods: Partial<LiveSessionSummary> = {},
  ): LiveSessionSummary {
    return {
      session_id: "sess-1",
      pid: 1,
      cwd,
      transcript_path: null,
      status: "busy",
      current_action: null,
      model: null,
      waiting_for: null,
      errored: false,
      stuck: false,
      idle_ms: 0,
      seq: 0,
      ...mods,
    };
  }

  function setLive(next: LiveSessionSummary[]) {
    act(() => liveStore.set(next));
  }

  beforeEach(() => {
    liveStore.set([]);
    glyphProbe.count = 0;
  });

  it("renders live dots only inside the matching project's row", () => {
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={noopSelect}
      />,
    );
    setLive([mkLive("/alpha/sub")]);

    const dot = screen.getByLabelText(/^Session /);
    const row = dot.closest('[role="option"]');
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getAllByTitle("/alpha").length)
      .toBeGreaterThan(0);
  });

  it("does not re-render any row when a live update matches no project", () => {
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={noopSelect}
      />,
    );
    const before = glyphProbe.count;
    setLive([mkLive("/elsewhere/entirely")]);
    // The parent re-renders (1 Glyph: the active sort arrow in the
    // header) but every memoized ProjectRow must bail — `live` stays
    // undefined and all other props are referentially stable, so
    // zero folder Glyphs re-render. Pre-memo this delta was 4.
    expect(glyphProbe.count - before).toBe(1);
  });

  it("re-renders only the matching row when a live session appears", () => {
    render(
      <ProjectsTable
        projects={fixtures}
        filter="all"
        selectedPath={null}
        onSelect={noopSelect}
      />,
    );
    const before = glyphProbe.count;
    setLive([mkLive("/alpha/sub")]);
    // Header sort arrow (1) + alpha's folder Glyph (1). Beta and
    // gamma rows must bail via React.memo. Pre-memo this delta was 4.
    expect(glyphProbe.count - before).toBe(2);
    expect(screen.getByLabelText(/^Session /)).toBeInTheDocument();
  });
});
