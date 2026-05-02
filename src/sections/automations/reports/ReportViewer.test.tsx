import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const invokeSpy = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invokeSpy(...a),
}));

import { ReportViewer } from "./ReportViewer";

describe("ReportViewer — modal markdown viewer", () => {
  beforeEach(() => {
    invokeSpy.mockReset();
  });

  it("renders nothing when path is null", () => {
    render(<ReportViewer path={null} onClose={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(invokeSpy).not.toHaveBeenCalled();
  });

  it("requests the file via templates_read_report when path is set", async () => {
    invokeSpy.mockResolvedValue("body");
    render(
      <ReportViewer
        path="/Users/x/.claudepot/reports/morning/2026-05-02.md"
        onClose={() => {}}
      />,
    );
    await waitFor(() =>
      expect(invokeSpy).toHaveBeenCalledWith("templates_read_report", {
        path: "/Users/x/.claudepot/reports/morning/2026-05-02.md",
      }),
    );
  });

  it("shows the file basename in the heading and the full path in the footer", async () => {
    invokeSpy.mockResolvedValue("anything");
    render(
      <ReportViewer
        path="/Users/x/.claudepot/reports/morning/2026-05-02.md"
        onClose={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "2026-05-02.md" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByText("/Users/x/.claudepot/reports/morning/2026-05-02.md"),
    ).toBeInTheDocument();
  });

  it("shows a loading indicator until the file content arrives", async () => {
    let resolve!: (s: string) => void;
    invokeSpy.mockReturnValue(
      new Promise<string>((res) => {
        resolve = res;
      }),
    );
    render(<ReportViewer path="/r.md" onClose={() => {}} />);
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    resolve("done");
    await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
    expect(screen.getByText("done")).toBeInTheDocument();
  });

  it("surfaces a read error inline without crashing the modal", async () => {
    invokeSpy.mockRejectedValue(new Error("EACCES"));
    render(<ReportViewer path="/r.md" onClose={() => {}} />);
    await waitFor(() =>
      expect(screen.getByText(/Couldn’t read report/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/EACCES/)).toBeInTheDocument();
  });

  it("Close button triggers onClose", async () => {
    invokeSpy.mockResolvedValue("body");
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ReportViewer path="/r.md" onClose={onClose} />);
    await waitFor(() => expect(screen.getByText("body")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("re-fetches when path changes (different report selected)", async () => {
    invokeSpy.mockResolvedValue("first");
    const { rerender } = render(
      <ReportViewer path="/a.md" onClose={() => {}} />,
    );
    await waitFor(() => expect(screen.getByText("first")).toBeInTheDocument());
    expect(invokeSpy).toHaveBeenCalledWith("templates_read_report", { path: "/a.md" });

    invokeSpy.mockResolvedValue("second");
    rerender(<ReportViewer path="/b.md" onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText("second")).toBeInTheDocument());
    expect(invokeSpy).toHaveBeenCalledWith("templates_read_report", { path: "/b.md" });
  });
});
