import { useEffect, useRef, useState, useCallback } from "react";

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
  const actionItems = items.filter((i) => !i.separator);
  const [focusIdx, setFocusIdx] = useState(0);

  // Close on outside click
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [onClose]);

  // Keyboard: Escape, Arrow keys, Enter/Space
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          e.preventDefault();
          onClose();
          break;
        case "ArrowDown":
          e.preventDefault();
          setFocusIdx((i) => Math.min(i + 1, actionItems.length - 1));
          break;
        case "ArrowUp":
          e.preventDefault();
          setFocusIdx((i) => Math.max(i - 1, 0));
          break;
        case "Enter":
        case " ": {
          e.preventDefault();
          const item = actionItems[focusIdx];
          if (item && !item.disabled) {
            item.onClick();
            onClose();
          }
          break;
        }
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose, actionItems, focusIdx]);

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
      aria-label="Context menu"
    >
      {(() => {
        let actionIdx = 0;
        return items.map((item, i) =>
          item.separator ? (
            <div key={i} className="context-menu-separator" role="separator" />
          ) : (
            <button
              key={i}
              className={`context-menu-item ${item.danger ? "danger" : ""} ${actionIdx === focusIdx ? "focused" : ""}`}
              role="menuitem"
              tabIndex={actionIdx++ === focusIdx ? 0 : -1}
              disabled={item.disabled}
              onClick={() => handleItemClick(item)}
            >
              {item.label}
            </button>
          ),
        );
      })()}
    </div>
  );
}
