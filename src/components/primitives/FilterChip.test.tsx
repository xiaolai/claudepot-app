import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { FilterChip } from "./FilterChip";

describe("FilterChip", () => {
  it("renders label and reflects active state via aria-pressed", () => {
    const onToggle = vi.fn();
    render(
      <FilterChip active={false} onToggle={onToggle}>
        Errors
      </FilterChip>,
    );
    const btn = screen.getByRole("switch", { name: "Errors" });
    expect(btn).toHaveAttribute("aria-pressed", "false");
  });

  it("reflects active=true on aria-pressed", () => {
    render(
      <FilterChip active onToggle={() => {}}>
        Errors
      </FilterChip>,
    );
    expect(
      screen.getByRole("switch", { name: "Errors" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("fires onToggle on click", async () => {
    const onToggle = vi.fn();
    render(
      <FilterChip active={false} onToggle={onToggle}>
        Agents
      </FilterChip>,
    );
    await userEvent.click(screen.getByRole("switch", { name: "Agents" }));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("fires onToggle on Space and Enter", async () => {
    const onToggle = vi.fn();
    render(
      <FilterChip active={false} onToggle={onToggle}>
        Agents
      </FilterChip>,
    );
    const btn = screen.getByRole("switch", { name: "Agents" });
    btn.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard(" ");
    expect(onToggle).toHaveBeenCalledTimes(2);
  });

  it("renders count when > 0, hides when 0 (render-if-nonzero)", () => {
    const { rerender } = render(
      <FilterChip active={false} onToggle={() => {}} count={3}>
        Errors
      </FilterChip>,
    );
    expect(screen.getByRole("switch")).toHaveTextContent(/Errors\s*3/);

    rerender(
      <FilterChip active={false} onToggle={() => {}} count={0}>
        Errors
      </FilterChip>,
    );
    // Zero count must not render at all.
    expect(screen.getByRole("switch").textContent).toBe("Errors");
  });

  it("does not fire onToggle when disabled", async () => {
    const onToggle = vi.fn();
    render(
      <FilterChip active={false} onToggle={onToggle} disabled>
        Errors
      </FilterChip>,
    );
    await userEvent.click(screen.getByRole("switch"));
    expect(onToggle).not.toHaveBeenCalled();
  });
});
