import { useEffect, useRef, useCallback } from "react";

export interface ContextMenuItem {
  label: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  onClick: () => void;
}

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Escape
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  // Prevent menu from going off-screen
  useEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
      el.style.left = `${window.innerWidth - rect.width - 4}px`;
    }
    if (rect.bottom > window.innerHeight) {
      el.style.top = `${window.innerHeight - rect.height - 4}px`;
    }
  }, [x, y]);

  const handleItemClick = useCallback(
    (item: ContextMenuItem) => {
      if (item.disabled) return;
      item.onClick();
      onClose();
    },
    [onClose],
  );

  return (
    <div
      ref={menuRef}
      className="context-menu"
      style={{ left: x, top: y }}
      role="menu"
    >
      {items.map((item, i) =>
        item.separator ? (
          <div key={i} className="context-menu-separator" role="separator" />
        ) : (
          <button
            key={i}
            className={`context-menu-item ${item.danger ? "danger" : ""}`}
            role="menuitem"
            disabled={item.disabled}
            onClick={() => handleItemClick(item)}
          >
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}
