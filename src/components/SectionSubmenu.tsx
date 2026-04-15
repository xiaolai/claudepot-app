import { useEffect, useRef } from "react";

export interface SubmenuItem {
  id: string;
  label: string;
  onSelect: () => void;
}

/**
 * Popover anchored to a rail icon. No section uses this yet — added
 * alongside the rail so future sections (Settings, MCP, …) can
 * declare sub-routes without re-architecting the shell.
 *
 * Click-outside and Escape both dismiss. Focus moves to the first
 * item on mount so keyboard users can Tab through without grabbing
 * the mouse.
 */
export function SectionSubmenu({
  items,
  anchor,
  onClose,
}: {
  items: readonly SubmenuItem[];
  anchor: { top: number; left: number };
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    const onClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onClick);
    };
  }, [onClose]);

  useEffect(() => {
    const first = ref.current?.querySelector<HTMLButtonElement>("button");
    first?.focus();
  }, []);

  return (
    <div
      ref={ref}
      role="menu"
      className="section-submenu"
      style={{ top: anchor.top, left: anchor.left }}
    >
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          className="section-submenu-item"
          onClick={() => {
            item.onSelect();
            onClose();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
