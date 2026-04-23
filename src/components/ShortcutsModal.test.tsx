import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ShortcutsModal } from "./ShortcutsModal";

describe("ShortcutsModal — navigation section rows", () => {
  it("lists all 6 ⌘N navigation bindings after Config insertion", () => {
    render(<ShortcutsModal onClose={() => {}} />);
    // Every primary section gets one row in the Navigation group. After
    // Config lands at position 4, ⌘1 → Accounts … ⌘6 → Settings.
    expect(screen.getByText("Accounts")).toBeInTheDocument();
    expect(screen.getByText("Projects")).toBeInTheDocument();
    expect(screen.getByText("Sessions")).toBeInTheDocument();
    expect(screen.getByText("Config")).toBeInTheDocument();
    expect(screen.getByText("Keys")).toBeInTheDocument();
    // "Settings" appears twice (⌘6 + standard ⌘,); both rows are fine.
    expect(screen.getAllByText(/Settings/).length).toBeGreaterThanOrEqual(2);
  });
});
