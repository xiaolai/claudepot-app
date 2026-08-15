import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { EmptyState } from "./EmptyState";
import { NF } from "../../icons";

/**
 * The primitive that replaced five private `EmptyState` components.
 *
 * These assert the properties that made consolidation worth doing —
 * one shape, an explicit next-action decision, and an inline variant
 * that does not impose block chrome — rather than re-testing React.
 */
describe("EmptyState", () => {
  it("renders title, body and action together", () => {
    render(
      <EmptyState
        title="No agents yet"
        body="Scheduled runs appear here."
        action={<button type="button">Add agent</button>}
      />,
    );
    expect(screen.getByRole("heading", { name: "No agents yet" })).toBeInTheDocument();
    expect(screen.getByText("Scheduled runs appear here.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add agent" })).toBeInTheDocument();
  });

  /**
   * `action={null}` is the documented way to say "there genuinely is
   * no next step". It must render nothing rather than a placeholder —
   * the point is that the absence is visible in the CALLER, not on
   * screen.
   */
  it("renders no action chrome when the caller passes null", () => {
    render(<EmptyState body="Nothing matched." action={null} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("Nothing matched.")).toBeInTheDocument();
  });

  it("omits the heading when no title is given", () => {
    render(<EmptyState body="Nothing matched." action={null} />);
    expect(screen.queryByRole("heading")).toBeNull();
  });

  /**
   * The inline variant exists so a note inside a populated pane does
   * not get a dashed enclosure and 32px of padding. `HealthPane` grew
   * its own component precisely because the block shape was wrong
   * there.
   */
  it("inline lays out as a row, block as a padded column", () => {
    // Asserts flex-direction rather than the border. jsdom does not
    // resolve `var(--bw-hair)`, so BOTH variants' computed border is
    // `medium none rgba(0, 0, 0, 0)` and a border comparison can never
    // distinguish them — it fails whatever the component does. Layout
    // direction is the structural difference and jsdom can see it.
    const { container: inline } = render(
      <EmptyState variant="inline" body="No sections matched." action={null} />,
    );
    expect(
      getComputedStyle(inline.firstElementChild as HTMLElement).flexDirection,
    ).not.toBe("column");

    const { container: block } = render(
      <EmptyState body="No sections matched." action={null} />,
    );
    expect(
      getComputedStyle(block.firstElementChild as HTMLElement).flexDirection,
    ).toBe("column");
  });

  it("accepts a glyph and rich body content", () => {
    render(
      <EmptyState
        glyph={NF.info}
        body={
          <span>
            Nothing here — <code>run a harvest</code>
          </span>
        }
        action={null}
      />,
    );
    expect(screen.getByText("run a harvest")).toBeInTheDocument();
  });
});
