import { useRef } from "react";
import type { SectionDef } from "../sections/registry";

/**
 * 48px icon column on the left of the window. Each rail button maps
 * to one section in the registry. The active section gets
 * `aria-current="page"` + a visible fill; others are flat icon
 * buttons.
 *
 * Keyboard model (macOS sidebar idiom):
 *   - Tab enters the rail.
 *   - ArrowUp / ArrowDown move focus (and selection) between items.
 *   - Home / End jump to first / last.
 *   - ⌘1..⌘9 activates a section globally (bound in useSection).
 *
 * Icons are Lucide (stroke-based, defaults set via CSS on svg.lucide);
 * active state is rendered via CSS, not by swapping icons — simpler
 * and matches existing `.icon-btn` patterns.
 */
export function SectionRail({
  sections,
  active,
  onSelect,
}: {
  sections: readonly SectionDef[];
  active: string;
  onSelect: (id: string) => void;
}) {
  const navRef = useRef<HTMLElement>(null);
  const activeIndex = Math.max(
    0,
    sections.findIndex((s) => s.id === active),
  );

  const handleKeyDown = (
    e: React.KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
  ) => {
    let nextIndex = currentIndex;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowRight":
        nextIndex = (currentIndex + 1) % sections.length;
        break;
      case "ArrowUp":
      case "ArrowLeft":
        nextIndex =
          (currentIndex - 1 + sections.length) % sections.length;
        break;
      case "Home":
        nextIndex = 0;
        break;
      case "End":
        nextIndex = sections.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    const next = sections[nextIndex];
    if (!next) return;
    onSelect(next.id);
    // Move focus to the newly-active button so the caret follows the
    // user's keyboard intent — this is the macOS sidebar behavior.
    const nav = navRef.current;
    if (nav) {
      const target = nav.querySelectorAll<HTMLButtonElement>(
        "button.section-rail-item",
      )[nextIndex];
      target?.focus();
    }
  };

  return (
    <nav ref={navRef} className="section-rail" aria-label="Sections">
      {sections.map((s, i) => {
        const isActive = s.id === active;
        // ⌘1..⌘9 tooltip hint for the first nine sections.
        const shortcutHint =
          i < 9 ? ` (⌘${i + 1})` : "";
        return (
          <button
            key={s.id}
            type="button"
            className={`section-rail-item${isActive ? " active" : ""}`}
            aria-label={s.label}
            aria-current={isActive ? "page" : undefined}
            // Roving tabindex: only the active item is Tab-reachable;
            // arrows move focus between items. Matches ARIA APG
            // vertical-toolbar guidance.
            tabIndex={i === activeIndex ? 0 : -1}
            title={`${s.label}${shortcutHint}`}
            onClick={() => onSelect(s.id)}
            onKeyDown={(e) => handleKeyDown(e, i)}
          >
            {s.icon}
          </button>
        );
      })}
    </nav>
  );
}
