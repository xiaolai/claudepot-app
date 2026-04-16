import { describe, expect, it, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PendingJournalsBanner } from "../../components/PendingJournalsBanner";

beforeEach(() => {
  localStorage.clear();
});

const summary = (p: number, s: number, r = 0) => ({
  pending: p,
  stale: s,
  running: r,
});

describe("PendingJournalsBanner", () => {
  it("renders nothing when total is zero", () => {
    const { container } = render(
      <PendingJournalsBanner summary={summary(0, 0)} onOpen={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders singular label for total=1", () => {
    render(
      <PendingJournalsBanner summary={summary(1, 0)} onOpen={() => {}} />,
    );
    expect(screen.getByText(/1 pending rename journal/)).toBeInTheDocument();
  });

  it("renders plural label for total>1", () => {
    render(
      <PendingJournalsBanner summary={summary(1, 2)} onOpen={() => {}} />,
    );
    expect(screen.getByText(/3 pending rename journals/)).toBeInTheDocument();
  });

  it("adds .stale modifier when any stale entry exists", () => {
    render(
      <PendingJournalsBanner summary={summary(0, 1)} onOpen={() => {}} />,
    );
    const btn = screen.getByRole("button");
    expect(btn.className).toContain("stale");
  });

  it("uses neutral tone when only pending entries exist", () => {
    render(
      <PendingJournalsBanner summary={summary(3, 0)} onOpen={() => {}} />,
    );
    const btn = screen.getByRole("button");
    expect(btn.className).not.toContain("stale");
  });

  it("calls onOpen when clicked", async () => {
    const user = userEvent.setup();
    let opened = false;
    render(
      <PendingJournalsBanner
        summary={summary(2, 0)}
        onOpen={() => { opened = true; }}
      />,
    );
    await user.click(screen.getByRole("button"));
    expect(opened).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// useSection subRoute behavior — the Step 3 extension.
// ---------------------------------------------------------------------------

describe("useSection subRoute", () => {
  it("persists subRoute per-section so switching sections doesn't bleed state", async () => {
    const { useSection } = await import("../../hooks/useSection");
    function Harness() {
      const { section, subRoute, setSection, setSubRoute } = useSection(
        "accounts",
        ["accounts", "projects"] as const,
      );
      return (
        <div>
          <span data-testid="section">{section}</span>
          <span data-testid="subroute">{subRoute ?? "∅"}</span>
          <button onClick={() => setSection("projects")}>go projects</button>
          <button onClick={() => setSubRoute("repair")}>set repair</button>
          <button onClick={() => setSection("accounts")}>go accounts</button>
          <button onClick={() => setSection("projects")}>go projects again</button>
        </div>
      );
    }
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByText("go projects"));
    await user.click(screen.getByText("set repair"));
    expect(screen.getByTestId("subroute")).toHaveTextContent("repair");

    // Switch to accounts — repair subRoute should persist for projects.
    await user.click(screen.getByText("go accounts"));
    expect(screen.getByTestId("section")).toHaveTextContent("accounts");
    // accounts has no persisted subRoute, so it's null.
    expect(screen.getByTestId("subroute")).toHaveTextContent("∅");

    // Back to projects — repair should be restored from storage.
    await user.click(screen.getByText("go projects again"));
    expect(screen.getByTestId("subroute")).toHaveTextContent("repair");
  });

  it("setSection with explicit subRoute deep-links atomically", async () => {
    const { useSection } = await import("../../hooks/useSection");
    function Harness() {
      const { section, subRoute, setSection } = useSection(
        "accounts",
        ["accounts", "projects"] as const,
      );
      return (
        <div>
          <span data-testid="section">{section}</span>
          <span data-testid="subroute">{subRoute ?? "∅"}</span>
          <button onClick={() => setSection("projects", "repair")}>deep-link</button>
        </div>
      );
    }
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByText("deep-link"));
    expect(screen.getByTestId("section")).toHaveTextContent("projects");
    expect(screen.getByTestId("subroute")).toHaveTextContent("repair");
  });
});

// ---------------------------------------------------------------------------
// usePendingJournals polling + focus refresh.
// ---------------------------------------------------------------------------

describe("usePendingJournals", () => {
  it("reads count on mount and via focus events", async () => {
    let callCount = 0;
    const fakeCounts = [2, 5, 0];
    const getNext = () =>
      fakeCounts[Math.min(callCount++, fakeCounts.length - 1)];

    const original = globalThis;
    // Hoist a mock via vi.doMock BEFORE importing the hook.
    const { vi } = await import("vitest");
    vi.resetModules();
    vi.doMock("../../api", () => ({
      api: {
        repairStatusSummary: () =>
          Promise.resolve({ pending: getNext(), stale: 0, running: 0 }),
      },
    }));
    const { usePendingJournals } = await import("../../hooks/usePendingJournals");

    function Harness() {
      const { count } = usePendingJournals();
      return <span data-testid="count">{count === null ? "∅" : String(count)}</span>;
    }
    render(<Harness />);

    // Mount triggers first read (resolves async).
    await screen.findByText("2");

    // Focus triggers a second read.
    act(() => {
      original.window.dispatchEvent(new Event("focus"));
    });
    await screen.findByText("5");
  });
});
