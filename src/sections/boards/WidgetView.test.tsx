// WidgetView draws a resolved plan and must never present a partial
// render as complete. Every assertion here maps to a plan §7 guard that
// core already resolved — the risk this file covers is the *renderer*
// dropping the caveat on the floor.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WidgetView } from "./WidgetView";
import type { ResolvedWidget } from "../../types";

function widget(plan: ResolvedWidget["plan"]): ResolvedWidget {
  return { id: "w", title: { text: "Cost" }, plan };
}

describe("WidgetView", () => {
  it("renders an empty state with the reason core supplied", () => {
    render(
      <WidgetView
        widget={widget({ kind: "empty", reason: "every value in this series is empty" })}
      />,
    );
    expect(
      screen.getByText("every value in this series is empty"),
    ).toBeInTheDocument();
  });

  it("says how many points were dropped rather than truncating silently", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "line",
          points: [{ x: "a", y: 1 }],
          y_axis: { min: 0, max: 2, scale: { kind: "linear" }, padded: false },
          downsampled_from: 10000,
        })}
      />,
    );
    expect(screen.getByText(/showing 1 of 10000 points/)).toBeInTheDocument();
  });

  it("says when a log axis was refused, and why", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "line",
          points: [{ x: "a", y: 1 }],
          y_axis: {
            min: 0,
            max: 2,
            scale: {
              kind: "log_fell_back_to_linear",
              reason: "series contains zero or negative values",
            },
            padded: false,
          },
          downsampled_from: null,
        })}
      />,
    );
    // Silently drawing a linear chart when the spec said log is the
    // failure this guards.
    expect(screen.getByText(/log axis unavailable/)).toBeInTheDocument();
    expect(
      screen.getByText(/zero or negative values/),
    ).toBeInTheDocument();
  });

  it("says when a flat series had its range padded", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "line",
          points: [{ x: "a", y: 5 }],
          y_axis: { min: 4.5, max: 5.5, scale: { kind: "linear" }, padded: true },
          downsampled_from: null,
        })}
      />,
    );
    expect(screen.getByText(/range padded/)).toBeInTheDocument();
  });

  it("breaks the line at a gap instead of joining across it", () => {
    const { container } = render(
      <WidgetView
        widget={widget({
          kind: "line",
          points: [
            { x: "a", y: 1 },
            { x: "b", y: null },
            { x: "c", y: 3 },
          ],
          y_axis: { min: 0, max: 4, scale: { kind: "linear" }, padded: false },
          downsampled_from: null,
        })}
      />,
    );
    // Two separate paths, not one path spanning the hole. A single
    // path would draw a straight line through data that does not exist.
    expect(container.querySelectorAll("path")).toHaveLength(2);
  });

  it("reports collapsed bar categories", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "bar",
          points: [{ x: "a", y: 1 }],
          y_axis: { min: 0, max: 2, scale: { kind: "linear" }, padded: false },
          collapsed_categories: 160,
        })}
      />,
    );
    expect(screen.getByText(/160 smaller categories grouped/)).toBeInTheDocument();
  });

  it("reports a truncated table's true row count", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "table",
          headers: [{ text: "v" }],
          rows: [["1"]],
          total_rows: 600,
        })}
      />,
    );
    expect(screen.getByText(/showing 1 of 600 rows/)).toBeInTheDocument();
  });

  it("does not claim truncation when the table is complete", () => {
    render(
      <WidgetView
        widget={widget({
          kind: "table",
          headers: [{ text: "v" }],
          rows: [["1"]],
          total_rows: 1,
        })}
      />,
    );
    expect(screen.queryByText(/showing/)).not.toBeInTheDocument();
  });

  it("renders a null KPI as a dash, never as zero", () => {
    render(
      <WidgetView widget={widget({ kind: "kpi", value: null, sample_size: 0 })} />,
    );
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(screen.queryByText("0")).not.toBeInTheDocument();
  });

  it("renders a single-point series as a visible mark, not a blank chart", () => {
    // A one-point run emits only an SVG `M`, which paints nothing.
    const { container } = render(
      <WidgetView
        widget={widget({
          kind: "line",
          points: [{ x: "a", y: 5 }],
          y_axis: { min: 4.5, max: 5.5, scale: { kind: "linear" }, padded: true },
          downsampled_from: null,
        })}
      />,
    );
    const d = container.querySelector("path")?.getAttribute("d") ?? "";
    expect(d).toContain("L");
  });

  it("says a table is truncated at the RENDER limit, not the plan size", () => {
    // A 500-row plan renders 50 rows. Computing the note from
    // plan.rows.length claimed it was complete.
    const rows = Array.from({ length: 500 }, (_, i) => [String(i)]);
    render(
      <WidgetView
        widget={widget({
          kind: "table",
          headers: [{ text: "v" }],
          rows,
          total_rows: 500,
        })}
      />,
    );
    expect(screen.getByText(/showing 50 of 500 rows/)).toBeInTheDocument();
  });

  it("carries the full text of a truncated title in a tooltip", () => {
    const long = "x".repeat(200);
    render(
      <WidgetView
        widget={{
          id: "w",
          title: { text: "xxx…", full: long },
          plan: { kind: "empty", reason: "no rows yet" },
        }}
      />,
    );
    expect(screen.getByTitle(long)).toBeInTheDocument();
  });
});
