/**
 * The `Input` primitive's contract, most of which is about what it does
 * NOT do to its caller.
 *
 * The wrapper used to be a `<label>`. That claimed every click inside it
 * for the input — including clicks on anything interactive a caller put in
 * `suffix` — and left no way to hand out an `id` for an external
 * `<label htmlFor>` to point at. Both are exercised here so neither comes
 * back.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Input } from "./Input";
import { NF } from "../../icons";

describe("Input", () => {
  it("is controlled when given a value and uncontrolled when not", async () => {
    // Uncontrolled: the DOM node keeps what the user typed. The secret
    // field in Global → Config → Env Variables depends on this — a
    // controlled field would put the plaintext in React state.
    const { unmount } = render(<Input aria-label="free" />);
    const free = screen.getByLabelText("free") as HTMLInputElement;
    await userEvent.type(free, "abc");
    expect(free.value).toBe("abc");
    unmount();

    // Controlled: the value prop wins, so a parent that ignores onChange
    // pins the field.
    render(<Input aria-label="pinned" value="fixed" onChange={() => {}} />);
    const pinned = screen.getByLabelText("pinned") as HTMLInputElement;
    await userEvent.type(pinned, "abc");
    expect(pinned.value).toBe("fixed");
  });

  it("does not wrap the field in a label, so an external one can own it", () => {
    const { container } = render(
      <>
        <label htmlFor="my-field">Outer label</label>
        <Input id="my-field" value="" onChange={() => {}} />
      </>,
    );
    // Exactly one label in the tree — the caller's.
    expect(container.querySelectorAll("label")).toHaveLength(1);
    // …and it resolves to the input.
    expect(screen.getByLabelText("Outer label").tagName).toBe("INPUT");
  });

  it("forwards native attributes the hand-written prop list used to drop", () => {
    render(
      <Input
        aria-label="native"
        name="token"
        required
        autoComplete="off"
        inputMode="numeric"
        maxLength={10}
        aria-invalid
      />,
    );
    const el = screen.getByLabelText("native");
    expect(el).toHaveAttribute("name", "token");
    expect(el).toBeRequired();
    expect(el).toHaveAttribute("autocomplete", "off");
    expect(el).toHaveAttribute("inputmode", "numeric");
    expect(el).toHaveAttribute("maxlength", "10");
    expect(el).toHaveAttribute("aria-invalid", "true");
  });

  it("gives the field an id even when the caller supplies none", () => {
    render(<Input aria-label="auto" />);
    expect(screen.getByLabelText("auto").id).not.toBe("");
  });

  it("focuses the field when the leading glyph is clicked", async () => {
    render(<Input glyph={NF.search} aria-label="searchable" />);
    const el = screen.getByLabelText("searchable");
    expect(el).not.toHaveFocus();
    // The glyph is decorative, so click it by position in the wrapper.
    const glyph = el.parentElement!.querySelector('[aria-hidden="true"]')!;
    await userEvent.click(glyph);
    expect(el).toHaveFocus();
  });

  it("still calls a caller's own focus and blur handlers", async () => {
    const onFocus = vi.fn();
    const onBlur = vi.fn();
    render(<Input aria-label="watched" onFocus={onFocus} onBlur={onBlur} />);
    const el = screen.getByLabelText("watched");
    await userEvent.click(el);
    expect(onFocus).toHaveBeenCalled();
    await userEvent.tab();
    expect(onBlur).toHaveBeenCalled();
  });
});
