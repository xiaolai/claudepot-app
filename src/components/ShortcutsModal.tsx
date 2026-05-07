import { useId, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Modal, ModalHeader, ModalBody } from "./primitives/Modal";
import { Kbd } from "./primitives/Kbd";

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
 * Global shortcut reference. Mounted at the shell level so it's
 * reachable from every section (⌘/ or the palette entry). Static
 * content — kept in sync with handlers by convention since shortcut
 * owners are spread across useSection, useGlobalShortcuts, App.tsx,
 * Modal, CommandPalette, and SidebarLiveStrip.
 */
export function ShortcutsModal({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const titleId = useId();

  const groups: ShortcutGroup[] = useMemo(() => [
    {
      title: t("shortcuts.group.navigation"),
      items: [
        { keys: ["⌘", "1"], label: t("shortcuts.nav.accounts") },
        { keys: ["⌘", "2"], label: t("shortcuts.nav.projects") },
        { keys: ["⌘", "3"], label: t("shortcuts.nav.sessions") },
        { keys: ["⌘", "4"], label: t("shortcuts.nav.config") },
        { keys: ["⌘", "5"], label: t("shortcuts.nav.keys") },
        { keys: ["⌘", "6"], label: t("shortcuts.nav.settings") },
        { keys: ["⌘", ","], label: t("shortcuts.nav.settingsStd") },
      ],
    },
    {
      title: t("shortcuts.group.global"),
      items: [
        { keys: ["⌘", "K"], label: t("shortcuts.act.palette") },
        { keys: ["⌘", "/"], label: t("shortcuts.act.shortcuts") },
        { keys: ["⌘", "R"], label: t("shortcuts.act.refresh") },
        { keys: ["⌘", "N"], label: t("shortcuts.act.add"), scope: t("shortcuts.scope.accounts") },
        { keys: ["⌘", "F"], label: t("shortcuts.act.filter") },
        { keys: ["⌘", "⇧", "C"], label: t("shortcuts.act.copyEmail"), scope: t("shortcuts.scope.accounts") },
        { keys: ["⌘", "⇧", "L"], label: t("shortcuts.act.liveSessions") },
      ],
    },
    {
      title: t("shortcuts.group.modals"),
      items: [
        { keys: ["Esc"], label: t("shortcuts.modal.close") },
        { keys: ["Tab"], label: t("shortcuts.modal.focus") },
      ],
    },
    {
      title: t("shortcuts.group.palette"),
      items: [
        { keys: ["↑", "↓"], label: t("shortcuts.pal.move") },
        { keys: ["Enter"], label: t("shortcuts.pal.run") },
        { keys: ["Esc"], label: t("shortcuts.pal.close") },
      ],
    },
    {
      title: t("shortcuts.group.sessions"),
      items: [
        { keys: ["j"], label: t("shortcuts.ss.next") },
        { keys: ["k"], label: t("shortcuts.ss.prev") },
        { keys: ["Enter"], label: t("shortcuts.ss.open") },
      ],
    },
  ], [t]);

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={titleId}>
      <ModalHeader title={t("shortcuts.title")} id={titleId} onClose={onClose} />
      <ModalBody>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: "var(--sp-24) var(--sp-32)",
          }}
        >
          {groups.map((g) => (
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
          {t("shortcuts.footer")}
        </p>
      </ModalBody>
    </Modal>
  );
}
