import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";

import { FilterPicker, type PickerOption } from "./FilterPicker";

const OPTIONS: PickerOption[] = [
  { value: "/a/alpha", label: "alpha", detail: "/a/alpha" },
  { value: "/b/beta", label: "beta", detail: "/b/beta" },
  { value: "/c/gamma", label: "gamma", detail: "/c/gamma" },
];

/** The picker's query is controlled, so tests need a host that owns it. */
function Host({
  onChange = () => {},
  pinned,
  options = OPTIONS,
}: {
  onChange?: (o: PickerOption) => void;
  pinned?: PickerOption | null;
  options?: PickerOption[];
}) {
  const [query, setQuery] = useState("");
  const [value, setValue] = useState<string | null>(options[0]?.value ?? null);
  return (
    <FilterPicker
      options={options}
      pinned={pinned}
      value={value}
      onChange={(o) => {
        setValue(o.value);
        onChange(o);
      }}
      query={query}
      onQueryChange={setQuery}
      inputAriaLabel="Filter"
      listAriaLabel="Options"
      emptyText="Nothing matches"
    />
  );
}

const names = () => screen.getAllByRole("option").map((o) => o.textContent);

describe("FilterPicker", () => {
  it("lists every option before any query", () => {
    render(<Host />);
    expect(names()).toHaveLength(3);
  });

  it("filters on label and on detail", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.type(screen.getByRole("combobox"), "beta");
    expect(names()).toEqual(["beta/b/beta"]);

    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "/c/");
    expect(names()).toEqual(["gamma/c/gamma"]);
  });

  it("ranks a substring hit above a scattered subsequence", async () => {
    const user = userEvent.setup();
    render(
      <Host
        options={[
          // "ama" is a subsequence of "alpha-manifest" but a substring
          // of "gamma-tool" — the substring must come first.
          { value: "/1", label: "alpha-manifest" },
          { value: "/2", label: "gamma-tool" },
        ]}
      />,
    );
    await user.type(screen.getByRole("combobox"), "ama");
    expect(names()).toEqual(["gamma-tool", "alpha-manifest"]);
  });

  it("shows the empty text when nothing matches", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.type(screen.getByRole("combobox"), "zzzz");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("Nothing matches")).toBeInTheDocument();
  });

  it("keeps the pinned row first and exempt from filtering", async () => {
    const user = userEvent.setup();
    const pinned = { value: "pin", label: "Use this", detail: "/typed/path" };
    render(<Host pinned={pinned} />);
    expect(names()[0]).toBe("Use this/typed/path");

    await user.type(screen.getByRole("combobox"), "zzzz");
    // Every real option filtered out; the pinned row survives.
    expect(names()).toEqual(["Use this/typed/path"]);
  });

  it("arrow keys move the cursor and Enter picks the active row", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    const input = screen.getByRole("combobox");
    input.focus();

    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ value: "/c/gamma" }),
    );

    await user.keyboard("{ArrowUp}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/b/beta" }),
    );
  });

  it("Home and End jump to the ends", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    screen.getByRole("combobox").focus();

    await user.keyboard("{End}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/c/gamma" }),
    );
    await user.keyboard("{Home}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/a/alpha" }),
    );
  });

  it("the cursor never runs off either end of the list", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    screen.getByRole("combobox").focus();

    await user.keyboard("{ArrowUp}{ArrowUp}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/a/alpha" }),
    );
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/c/gamma" }),
    );
  });

  it("the row announced as active is the row Enter picks", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    const input = screen.getByRole("combobox");
    input.focus();
    await user.keyboard("{ArrowDown}");

    const activeId = input.getAttribute("aria-activedescendant");
    const active = screen.getAllByRole("option").find((o) => o.id === activeId);
    expect(active).toBeDefined();

    await user.keyboard("{Enter}");
    expect(onChange).toHaveBeenCalledTimes(1);
    const picked = onChange.mock.calls[0][0] as PickerOption;
    expect(active?.textContent).toContain(picked.label);
  });

  it("a filtered list resets the cursor to the top", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    const input = screen.getByRole("combobox");
    input.focus();

    // Move the cursor past where the shorter result set will end, then
    // narrow. Without the reset, Enter would pick nothing (or the wrong
    // row) because the index outlives the list it indexed into.
    await user.keyboard("{ArrowDown}{ArrowDown}");
    await user.type(input, "beta");
    await user.keyboard("{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: "/b/beta" }),
    );
  });

  it("clicking a row picks it", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Host onChange={onChange} />);
    await user.click(screen.getByRole("option", { name: /gamma/ }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ value: "/c/gamma" }),
    );
  });

  it("marks the picked row with aria-selected, independent of the cursor", async () => {
    const user = userEvent.setup();
    render(<Host />);
    screen.getByRole("combobox").focus();
    await user.keyboard("{ArrowDown}");
    // Cursor is on row 2; the picked row is still row 1.
    const selected = screen
      .getAllByRole("option")
      .filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0].textContent).toContain("alpha");
  });

  it("leaves Escape to the enclosing dialog", async () => {
    const onEscape = vi.fn();
    const user = userEvent.setup();
    render(
      <div onKeyDown={(e) => e.key === "Escape" && onEscape()}>
        <Host />
      </div>,
    );
    screen.getByRole("combobox").focus();
    await user.keyboard("{Escape}");
    expect(onEscape).toHaveBeenCalled();
  });
});
