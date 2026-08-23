import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Body, isSearching } from "./transcriptAtoms";

const MD = "Released. **v0.9.50 is building.**";

function body(props: Partial<Parameters<typeof Body>[0]> = {}) {
  const { container } = render(
    <Body text={MD} searchTerm="" clamp={9999} {...props} />,
  );
  return container;
}

describe("Body — when prose renders as markdown", () => {
  it("renders markdown for a prose turn with no search running", () => {
    // The ordinary case, and the one that was broken: the asterisks
    // used to be on screen.
    expect(body().querySelector("strong")?.textContent).toBe(
      "v0.9.50 is building.",
    );
  });

  it("keeps a tool payload verbatim", () => {
    // `mono` marks a command's arguments or its stdout. Markdown would
    // corrupt it — a shell comment becomes a heading, a glob becomes
    // emphasis — and the one thing a reader needs from output is that it
    // is what the command printed.
    const c = body({ mono: true, text: "# not a heading\n*.log  *.tmp" });
    expect(c.querySelector("h1")).toBeNull();
    expect(c.querySelector("em")).toBeNull();
    expect(c.textContent).toContain("# not a heading");
    expect(c.textContent).toContain("*.log");
  });

  it("gives way to search highlighting while a search is running", () => {
    // A match you can find beats a bold you can read. `highlight` marks
    // matches by splitting the raw string, which cannot be done through
    // a rendered element tree without walking it.
    const c = body({ searchTerm: "building" });
    expect(c.querySelector("mark")?.textContent).toBe("building");
    expect(c.querySelector("strong")).toBeNull();
  });

  it("still renders markdown when the search term is too short to be one", () => {
    // A one-character term does not highlight, so it must not suppress
    // rendering either — the two decisions read the same predicate.
    expect(body({ searchTerm: "b" }).querySelector("strong")).not.toBeNull();
    expect(body({ searchTerm: "   " }).querySelector("strong")).not.toBeNull();
  });

  it("keeps the clamp and its toggle", () => {
    // The cap exists so one enormous answer cannot lock the pane. It
    // slices the raw string, so a trimmed body can cut a fence open —
    // expanding restores it.
    const long = `${"a".repeat(50)} **bold**`;
    const c = body({ text: long, clamp: 20 });
    expect(c.textContent).not.toContain("bold");
    expect(screen.getByRole("button")).toBeInTheDocument();
  });
});

describe("isSearching", () => {
  it("is the one answer to whether a search is running", () => {
    // Three places used to decide this independently and the third
    // skipped the trim, so `" a "` was a search to one and not to
    // another.
    expect(isSearching("")).toBe(false);
    expect(isSearching("a")).toBe(false);
    expect(isSearching(" a ")).toBe(false);
    expect(isSearching("ab")).toBe(true);
    expect(isSearching("  ab  ")).toBe(true);
  });
});
