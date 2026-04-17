import type { SectionDef } from "../sections/registry";

/**
 * 48px icon column on the left of the window. Each rail button maps
 * to one section in the registry. The active section gets
 * `aria-current="page"` + a visible fill; others are flat icon
 * buttons.
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
  return (
    <nav className="section-rail" aria-label="Sections">
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
            title={`${s.label}${shortcutHint}`}
            onClick={() => onSelect(s.id)}
          >
            {s.icon}
          </button>
        );
      })}
    </nav>
  );
}
