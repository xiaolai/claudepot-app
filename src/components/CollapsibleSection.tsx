import { useState, useCallback, useEffect, useRef } from "react";
import { ChevronRight } from "lucide-react";

export function CollapsibleSection({
  title,
  defaultOpen = false,
  forceOpen,
  children,
}: {
  title: string;
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
      >
        <ChevronRight
          size={12}
          strokeWidth={2}
          className={`collapsible-chevron ${open ? "open" : ""}`}
        />
        <span className="collapsible-section-title">{title}</span>
      </button>
      {open && <div className="collapsible-section-body">{children}</div>}
    </div>
  );
}
