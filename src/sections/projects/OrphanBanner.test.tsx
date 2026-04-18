import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { OrphanBanner } from "./OrphanBanner";
import type { OrphanedProject } from "../../types";

function mk(overrides: Partial<OrphanedProject> = {}): OrphanedProject {
  return {
    slug: "-was-a-worktree",
    cwdFromTranscript: "/was/a/worktree",
    sessionCount: 3,
    totalSizeBytes: 2_500_000,
    suggestedAdoptionTarget: null,
    ...overrides,
  };
}

describe("OrphanBanner", () => {
  it("renders nothing when no orphans exist", () => {
    const { container } = render(
      <OrphanBanner orphans={[]} onAdopt={() => {}} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows singular copy for exactly one orphan", () => {
    render(<OrphanBanner orphans={[mk({ sessionCount: 1 })]} onAdopt={() => {}} />);
    const header = screen.getByText(/^1 orphaned project/);
    expect(header).toBeInTheDocument();
    expect(header.textContent).toMatch(/1 session/);
    // Not plural
    expect(header.textContent).not.toMatch(/1 sessions/);
  });

  it("aggregates session counts and sizes across multiple orphans", () => {
    render(
      <OrphanBanner
        orphans={[
          mk({ slug: "a", sessionCount: 2, totalSizeBytes: 1_000_000 }),
          mk({ slug: "b", sessionCount: 5, totalSizeBytes: 4_000_000 }),
        ]}
        onAdopt={() => {}}
      />,
    );
    // 2 orphans, 7 sessions, ~5 MB
    expect(screen.getByText(/2 orphaned projects/)).toBeInTheDocument();
    expect(screen.getByText(/7 sessions/)).toBeInTheDocument();
  });

  it("fires onAdopt when the action button is clicked", () => {
    const onAdopt = vi.fn();
    render(<OrphanBanner orphans={[mk()]} onAdopt={onAdopt} />);
    fireEvent.click(screen.getByRole("button", { name: /review.*adopt/i }));
    expect(onAdopt).toHaveBeenCalledTimes(1);
  });

  it("uses role=alert for assistive technology (persistent state)", () => {
    render(<OrphanBanner orphans={[mk()]} onAdopt={() => {}} />);
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
