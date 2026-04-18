import { useState, useCallback, useEffect, useRef } from "react";
import { Icon } from "./Icon";

export function CollapsibleSection({
  title,
  titleSuffix,
  defaultOpen = false,
  forceOpen,
  children,
}: {
  title: string;
  /** Optional inline node rendered alongside the title (e.g. a stale
   *  indicator chip). Part of the toggle button so clicking still works. */
  titleSuffix?: React.ReactNode;
  defaultOpen?: boolean;
  /** When true, section auto-opens (e.g. on anomaly). */
  forceOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const forcedRef = useRef(false);

  // Auto-expand on anomaly, but only once per forceOpen transition
  useEffect(() => {
    if (forceOpen && !forcedRef.current) {
      setOpen(true);
      forcedRef.current = true;
    }
    if (!forceOpen) {
      forcedRef.current = false;
    }
  }, [forceOpen]);

  const toggle = useCallback(() => setOpen((p) => !p), []);

  return (
    <div className="collapsible-section">
      <button
        className="collapsible-section-toggle"
        onClick={toggle}
        aria-expanded={open}
        title={`${open ? "Collapse" : "Expand"} ${title}`}
      >
        <Icon
          name="chevron-right"
          size={12}
          className={`collapsible-chevron ${open ? "open" : ""}`}
        />
        <span className="collapsible-section-title">{title}</span>
        {titleSuffix}
      </button>
      {open && <div className="collapsible-section-body">{children}</div>}
    </div>
  );
}
