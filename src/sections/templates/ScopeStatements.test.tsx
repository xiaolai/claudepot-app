import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ScopeStatements } from "./ScopeStatements";
import type { TemplateScopeDto } from "../../types";

describe("ScopeStatements — trust-boundary surface", () => {
  const scope: TemplateScopeDto = {
    reads: "Disk free space, large directories on this Mac.",
    writes: "A markdown file under ~/.claudepot/reports/.",
    could_change: "Nothing — this is read-only.",
    network: "None.",
  };

  it("renders all four scope rows verbatim — no paraphrasing", () => {
    render(<ScopeStatements scope={scope} />);
    expect(
      screen.getByText("Disk free space, large directories on this Mac."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("A markdown file under ~/.claudepot/reports/."),
    ).toBeInTheDocument();
    expect(screen.getByText("Nothing — this is read-only.")).toBeInTheDocument();
    expect(screen.getByText("None.")).toBeInTheDocument();
  });

  it("labels the four rows with stable headings (Reads/Writes/Changes/Network)", () => {
    render(<ScopeStatements scope={scope} />);
    expect(screen.getByText("Reads")).toBeInTheDocument();
    expect(screen.getByText("Writes")).toBeInTheDocument();
    expect(screen.getByText("Changes")).toBeInTheDocument();
    expect(screen.getByText("Network")).toBeInTheDocument();
  });

  it("renders empty strings without collapsing the row", () => {
    render(
      <ScopeStatements
        scope={{ reads: "", writes: "", could_change: "", network: "" }}
      />,
    );
    expect(screen.getByText("Reads")).toBeInTheDocument();
    expect(screen.getByText("Writes")).toBeInTheDocument();
    expect(screen.getByText("Changes")).toBeInTheDocument();
    expect(screen.getByText("Network")).toBeInTheDocument();
  });
});
