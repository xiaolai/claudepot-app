/**
 * The horizontal inset is the container's decision, not the label's.
 *
 * `SectionLabel` shipped with a `var(--sp-14)` horizontal padding that
 * was wrong nearly everywhere it was used. Measured against the running
 * app, it put every heading 14px right of its own content in Retention
 * (×4), MCP (×2), Knowledge (×3), Updates (×1) and Remote (×6) — and
 * 6px off the rows in the one place an inset was genuinely wanted,
 * because the sidebar's gutter is 8px, not 14. Three call sites were
 * already correcting it by hand with
 * `style={{ paddingLeft: 0, paddingRight: 0 }}`.
 *
 * None of that was visible to any check in this repo: layout is invisible
 * to `tsc`, to `check:classes`, and to jsdom, which has no layout engine.
 * What jsdom CAN hold is the declared contract — which is what these
 * assert, so a future change to the default is a failing test rather
 * than a silent 14px shift across a dozen surfaces.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SectionLabel } from "./SectionLabel";

const padding = (el: HTMLElement) => (el.closest("div") as HTMLElement).style.padding;

describe("SectionLabel", () => {
  it("is flush by default, so it starts where its content starts", () => {
    render(<SectionLabel>Devices</SectionLabel>);
    const pad = padding(screen.getByText("Devices"));
    // Vertical rhythm is the label's own business; horizontal is not.
    expect(pad).toContain("var(--sp-12)");
    expect(pad).toContain("var(--sp-6)");
    expect(pad).toBe("var(--sp-12) 0 var(--sp-6)");
  });

  it("takes an inset from a container whose rows are themselves inset", () => {
    // The sidebar live strip: its listbox is `padding: 0 var(--sp-8)`,
    // so the label matches the rows it names rather than the panel edge.
    render(<SectionLabel inset="var(--sp-8)">Live</SectionLabel>);
    expect(padding(screen.getByText("Live"))).toBe("var(--sp-12) var(--sp-8) var(--sp-6)");
  });

  it("still lets a caller override the padding outright", () => {
    // `AppSidebar` sets its own, because it wraps the label in a padded
    // block and needs the label to sit inside that rather than beside it.
    render(<SectionLabel style={{ padding: "0 var(--sp-4) var(--sp-6)" }}>Swap</SectionLabel>);
    expect(padding(screen.getByText("Swap"))).toBe("0 var(--sp-4) var(--sp-6)");
  });

  it("renders right-aligned content beside the label", () => {
    render(<SectionLabel right={<span>7</span>}>Devices</SectionLabel>);
    expect(screen.getByText("Devices")).toBeInTheDocument();
    expect(screen.getByText("7")).toBeInTheDocument();
  });
});
