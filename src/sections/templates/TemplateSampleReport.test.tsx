import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { TemplateSampleReport } from "./TemplateSampleReport";

const sampleSpy = vi.fn();
vi.mock("../../api", () => ({
  api: {
    templatesSampleReport: (...args: unknown[]) => sampleSpy(...args),
  },
}));

describe("TemplateSampleReport — preview before install", () => {
  beforeEach(() => {
    sampleSpy.mockReset();
  });

  it("shows a loading state until the bundled markdown resolves", async () => {
    let resolve!: (s: string) => void;
    sampleSpy.mockReturnValue(
      new Promise<string>((res) => {
        resolve = res;
      }),
    );
    render(<TemplateSampleReport templateId="it.morning-health-check" />);
    expect(screen.getByText("Loading sample…")).toBeInTheDocument();

    resolve("# Sample\n\nA quick read on how your Mac's doing.");
    await waitFor(() =>
      expect(screen.queryByText("Loading sample…")).toBeNull(),
    );
    expect(
      screen.getByText(/A quick read on how your Mac's doing\./),
    ).toBeInTheDocument();
  });

  it("renders the bundled markdown verbatim (no syntax highlighting; readability beats prettiness)", async () => {
    sampleSpy.mockResolvedValue("Subject: Morning health check\n\nAll green.");
    render(<TemplateSampleReport templateId="it.morning-health-check" />);
    await waitFor(() =>
      expect(screen.getByText(/Subject: Morning health check/)).toBeInTheDocument(),
    );
    // pre tag preserves whitespace
    const pre = screen.getByText(/Subject: Morning health check/);
    expect(pre.tagName).toBe("PRE");
  });

  it("surfaces a graceful 'No sample available' when the backend rejects", async () => {
    sampleSpy.mockRejectedValue(new Error("not bundled"));
    render(<TemplateSampleReport templateId="missing.id" />);
    await waitFor(() =>
      expect(screen.getByText(/No sample available/)).toBeInTheDocument(),
    );
  });

  it("re-fetches when templateId changes", async () => {
    sampleSpy.mockResolvedValue("first");
    const { rerender } = render(
      <TemplateSampleReport templateId="t.a" />,
    );
    await waitFor(() => expect(screen.getByText("first")).toBeInTheDocument());
    expect(sampleSpy).toHaveBeenCalledWith("t.a");

    sampleSpy.mockResolvedValue("second");
    rerender(<TemplateSampleReport templateId="t.b" />);
    await waitFor(() => expect(screen.getByText("second")).toBeInTheDocument());
    expect(sampleSpy).toHaveBeenCalledWith("t.b");
  });
});
