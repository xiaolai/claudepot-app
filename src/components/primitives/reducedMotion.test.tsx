/**
 * `design.md`'s accessibility floor commits to honouring
 * `prefers-reduced-motion`. Six CSS shards carry a
 * `@media (prefers-reduced-motion: reduce)` block, and between them
 * they could never cover the primitives: those animate from INLINE
 * styles, and an inline declaration beats any stylesheet rule that
 * carries no `!important`. So `Button`, `IconButton`, `SidebarItem`,
 * `FilterChip`, `modalParts` and the six settings toggles animated for
 * every user whatever the system setting said — while `accounts.css`
 * spent a reduced-motion block on `.collapsible-chevron`, a class
 * nothing renders.
 *
 * The fix zeroes the duration TOKENS in `tokens.css`, which reaches an
 * inline `transition: … var(--dur-base) …` because a custom property in
 * an inline style still resolves against the cascade. That only holds
 * while the primitives keep NAMING the tokens: a hardcoded
 * `transition: "background 120ms"` steps straight back outside the
 * override's reach with nothing to say so, and `EventsSection` had
 * exactly one of those.
 *
 * This file guards the rendered end of that contract — that the token
 * actually reaches `style.transition` on a mounted primitive. The
 * source-text end (the override exists, is ordered after the base
 * `--dur-*` declarations, and no call site anywhere uses a literal)
 * lives in `scripts/check-motion.mjs`, because reading a file needs
 * Node and the app's tsconfig carries no node types — and because
 * `?raw` yields an empty string under Vitest. That last one is
 * measured, not assumed: a probe importing `tokens.css?raw` here read
 * 0 characters, which is how an earlier CSS assertion in this repo
 * passed while reading nothing.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { SidebarItem } from "./SidebarItem";
import { FilterChip } from "./FilterChip";
import { NF } from "../../icons";

describe("primitives animate through duration tokens, not literals", () => {
  const cases: Array<[string, () => HTMLElement | null]> = [
    [
      "Button",
      () => render(<Button>Label</Button>).container.querySelector("button"),
    ],
    [
      "IconButton",
      () =>
        render(
          <IconButton glyph={NF.user} aria-label="Account" />,
        ).container.querySelector("button"),
    ],
    [
      "SidebarItem",
      () => render(<SidebarItem label="Item" />).container.querySelector("button"),
    ],
    [
      "FilterChip",
      () =>
        render(
          <FilterChip active={false} onToggle={() => {}}>
            Chip
          </FilterChip>,
        ).container.querySelector("button"),
    ],
  ];

  for (const [name, mount] of cases) {
    it(`${name} transitions through a var(--dur-…) token`, () => {
      const el = mount();
      expect(el).not.toBeNull();
      const transition = el!.style.transition;
      // An element that animates nothing would pass the token check
      // vacuously, so assert there is a transition to begin with.
      expect(transition).not.toBe("");
      expect(transition).toMatch(/var\(--dur-/);
      // A literal duration survives the token override — that is the
      // regression this catches.
      expect(transition).not.toMatch(/\d+\s*m?s\b/);
    });
  }
});
