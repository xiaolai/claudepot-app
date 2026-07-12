import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FoldableBubble, TURN_FOLD_CHARS } from "./transcriptAtoms";

/**
 * Turn-level fold: a long answer collapses to its header + a one-line
 * preview so a transcript stays scannable. Guards three things that are
 * easy to regress:
 *
 *  - short turns are untouched (no chevron, no behavior change),
 *  - long turns start folded but expand on click,
 *  - a turn matching the live search is NEVER folded. The chunk list is
 *    already filtered to matches, so folding one would hide the very
 *    hit the user searched for.
 */

const BODY = "THE-BODY-TEXT";

function renderBubble(foldText: string, searchTerm = "") {
  return render(
    <FoldableBubble
      side="left"
      tone="sunken"
      foldText={foldText}
      searchTerm={searchTerm}
      header={<span>Claude</span>}
    >
      <div>{BODY}</div>
    </FoldableBubble>,
  );
}

/** A turn comfortably over the fold threshold. */
function longText(marker = ""): string {
  return `${marker ? `${marker}\n` : ""}${"x".repeat(TURN_FOLD_CHARS + 50)}`;
}

describe("FoldableBubble", () => {
  it("leaves a short turn fully expanded with no fold control", () => {
    renderBubble("just a short answer");
    expect(screen.getByText(BODY)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /expand turn/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /collapse turn/i })).toBeNull();
  });

  it("folds a long turn by default, hiding the body behind a preview", () => {
    renderBubble(longText("first line of the answer"));
    // Body is folded away...
    expect(screen.queryByText(BODY)).toBeNull();
    // ...header still reads...
    expect(screen.getByText("Claude")).toBeInTheDocument();
    // ...and the preview shows the opening line + a size hint.
    expect(screen.getByText(/first line of the answer/)).toBeInTheDocument();
    expect(screen.getByText(/lines ·/i)).toBeInTheDocument();

    const toggle = screen.getByRole("button", { name: /expand turn/i });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("expands on chevron click and re-folds on a second click", async () => {
    renderBubble(longText());

    await userEvent.click(screen.getByRole("button", { name: /expand turn/i }));
    expect(screen.getByText(BODY)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /collapse turn/i }),
    ).toHaveAttribute("aria-expanded", "true");

    await userEvent.click(
      screen.getByRole("button", { name: /collapse turn/i }),
    );
    expect(screen.queryByText(BODY)).toBeNull();
  });

  it("expands when the folded preview itself is clicked", async () => {
    renderBubble(longText("click me to open"));
    await userEvent.click(screen.getByText(/click me to open/));
    expect(screen.getByText(BODY)).toBeInTheDocument();
  });

  it("never folds a long turn that matches the active search", () => {
    // The chunk list is already filtered to matches — folding this would
    // make the hit the user searched for invisible.
    renderBubble(longText("the needle is here"), "needle");
    expect(screen.getByText(BODY)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /expand turn/i })).toBeNull();
  });

  it("unfolds even when the hit is NOT in foldText (tool-result match)", () => {
    // Regression, found by driving the real app. `chunkMatchesSearch`
    // also matches tool inputs/results, which render inside `children`
    // and never appear in `foldText` (an AI turn's foldText is its prose
    // only). The old rule folded unless `foldText` contained the term,
    // so a turn surfaced by a tool-result hit stayed folded — search
    // hiding its own result. If a search is active, nothing folds.
    renderBubble(longText("prose that lacks the term"), "needle");
    expect(screen.getByText(BODY)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /expand turn/i })).toBeNull();
  });

  it("ignores a sub-2-char search term when deciding to fold", () => {
    // `normalizeDetailQuery` floors the query at 2 chars; a 1-char term
    // must not accidentally count as a match and unfold everything.
    renderBubble(longText("x marks it"), "x");
    expect(screen.queryByText(BODY)).toBeNull();
  });

  it("previews past a bare [thinking] marker line", () => {
    // Caught by driving the real app: SessionChunkView prefixes each
    // thinking block with a "[thinking]" line when building an AI
    // turn's text, so a turn that opens by thinking previewed as the
    // literal string "[thinking]" — a fold hint that says nothing.
    renderBubble(`[thinking]\nthe actual reasoning starts here\n${"x".repeat(TURN_FOLD_CHARS)}`);
    expect(screen.getByText(/the actual reasoning starts here/)).toBeInTheDocument();
    expect(screen.queryByText(/^\[thinking\]$/)).toBeNull();
  });
});
