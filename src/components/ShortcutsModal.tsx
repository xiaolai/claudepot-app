import { useId } from "react";
import { Modal, ModalHeader, ModalBody } from "./primitives/Modal";
import { Kbd } from "./primitives/Kbd";
import { sections } from "../sections/registry";

interface ShortcutBinding {
  keys: string[];
  label: string;
  scope?: string;
}

interface ShortcutGroup {
  title: string;
  items: ShortcutBinding[];
}

/**
 * ⌘1..⌘9 is bound in `useSection` by *position* in the section
 * registry, so the only correct way to document it is to read the
 * same list. The hand-written version drifted badly — it claimed ⌘3
 * was Sessions and ⌘4 was Config (neither is a section any more), put
 * Settings on ⌘6 when it is ⌘9, and never mentioned ⌘7..⌘9 at all.
 */
const NAVIGATION_ITEMS: ShortcutBinding[] = [
  ...sections.slice(0, 9).map((s, i) => ({
    keys: ["⌘", String(i + 1)],
    label: s.label,
  })),
  { keys: ["⌘", ","], label: "Settings (standard shortcut)" },
];

const GROUPS: ShortcutGroup[] = [
  {
    title: "Navigation",
    items: NAVIGATION_ITEMS,
  },
  {
    title: "Global actions",
    items: [
      { keys: ["⌘", "K"], label: "Open command palette" },
      { keys: ["⌘", "/"], label: "Show keyboard shortcuts" },
      { keys: ["⌘", "R"], label: "Refresh this section" },
      { keys: ["⌘", "N"], label: "Add account", scope: "Accounts" },
      // ⌘F was listed here as "Focus filter (where exposed)" but no
      // section ever wired it — the hook option existed and nothing
      // passed it. Documenting a shortcut that does nothing is worse
      // than not documenting it.
      { keys: ["⌘", "⇧", "C"], label: "Copy first matching email", scope: "Accounts" },
      { keys: ["⌘", "⇧", "L"], label: "Focus Live sessions strip" },
    ],
  },
  {
    title: "In modals",
    items: [
      { keys: ["Esc"], label: "Close dialog" },
      { keys: ["Tab"], label: "Cycle focus (trapped)" },
    ],
  },
  {
    title: "Command palette",
    items: [
      { keys: ["↑", "↓"], label: "Move selection" },
      { keys: ["Home"], label: "First result" },
      { keys: ["End"], label: "Last result" },
      { keys: ["Enter"], label: "Run selected" },
      { keys: ["Esc"], label: "Close palette" },
    ],
  },
  {
    title: "Live sessions strip",
    items: [
      { keys: ["j"], label: "Next session" },
      { keys: ["k"], label: "Previous session" },
      { keys: ["Enter"], label: "Open focused session" },
    ],
  },
];

/**
 * Global shortcut reference. Mounted at the shell level so it's
 * reachable from every section (⌘/ or the palette entry). Static
 * content — kept in sync with handlers by convention since shortcut
 * owners are spread across useSection, useGlobalShortcuts, App.tsx,
 * Modal, CommandPalette, and SidebarLiveStrip.
 */
export function ShortcutsModal({ onClose }: { onClose: () => void }) {
  const titleId = useId();
  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={titleId}>
      <ModalHeader title="Keyboard shortcuts" id={titleId} onClose={onClose} />
      <ModalBody>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: "var(--sp-24) var(--sp-32)",
          }}
        >
          {GROUPS.map((g) => (
            <section key={g.title}>
              <h3
                className="mono-cap"
                style={{
                  fontSize: "var(--fs-2xs)",
                  fontWeight: 500,
                  color: "var(--fg-muted)",
                  letterSpacing: "0.05em",
                  margin: "0 0 var(--sp-8) 0",
                }}
              >
                {g.title}
              </h3>
              <ul
                style={{
                  listStyle: "none",
                  padding: 0,
                  margin: 0,
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--sp-6)",
                }}
              >
                {g.items.map((it, i) => (
                  <li
                    key={i}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: "var(--sp-8)",
                      fontSize: "var(--fs-xs)",
                      color: "var(--fg)",
                    }}
                  >
                    <span style={{ flex: 1 }}>
                      {it.label}
                      {it.scope && (
                        <span
                          style={{
                            color: "var(--fg-faint)",
                            marginLeft: "var(--sp-6)",
                          }}
                        >
                          · {it.scope}
                        </span>
                      )}
                    </span>
                    <span
                      style={{
                        display: "inline-flex",
                        gap: "var(--sp-3)",
                        flexShrink: 0,
                      }}
                    >
                      {it.keys.map((k, ki) => (
                        <Kbd key={ki}>{k}</Kbd>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <p
          style={{
            marginTop: "var(--sp-20)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          Shortcuts are suppressed while typing in a text field or while
          a dialog is open.
        </p>
      </ModalBody>
    </Modal>
  );
}
