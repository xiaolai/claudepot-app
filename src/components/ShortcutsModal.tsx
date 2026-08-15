import { useId } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { Modal, ModalHeader, ModalBody } from "./primitives/Modal";
import { Kbd } from "./primitives/Kbd";
import { useEnabledSections } from "../hooks/useEnabledSections";
import { sectionNumber } from "../lib/shortcutBindings";

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
/**
 * Built per render, NOT at module load.
 *
 * A module-level `const` captured the enabled sections once at import,
 * so toggling an optional section never updated this modal until a
 * reload — the documentation silently disagreed with the bindings,
 * which is the exact drift this file was written to stop, one level up.
 */
function navigationItems(
  enabled: readonly { id: string; label: string }[],
  tc: TFunction<"components">,
): ShortcutBinding[] {
  // Numbers come from `sectionNumber` — position in the FULL registry,
  // the same source `useSection` binds against. Deriving them from the
  // *enabled* list (this function's previous shape) is what let the
  // documentation and the binding disagree the moment an optional
  // section was toggled: Boards ships off and sits ninth, so enabling
  // it moved Settings off ⌘9, and both this modal and the hook
  // silently agreed on the wrong thing together.
  //
  // Still filtered by `enabled`: a switched-off section's number is
  // reserved but inert, and documenting a dead key is the ⌘F mistake.
  const numbered = enabled
    .map((s) => ({ n: sectionNumber(s.id), label: s.label }))
    .filter((x): x is { n: number; label: string } => x.n !== null)
    .sort((a, b) => a.n - b.n)
    .map((x) => ({ keys: ["⌘", String(x.n)], label: x.label }));

  return [
    ...numbered,
    { keys: ["⌘", ","], label: tc("shortcuts.settingsStandard") },
    { keys: ["⌃", "⌥", "⌘", "B"], label: tc("shortcuts.toggleBoards") },
  ];
}

/**
 * Built per render for the same reason `navigationItems` is: a
 * module-level `const` would freeze the copy at import time, so a
 * language switch would leave the modal documenting the previous
 * language until a reload.
 *
 * `scopeAccounts` resolves from the **shell** catalog — the scope is
 * a section name, and the section labels already live there.
 */
function otherGroups(
  tc: TFunction<"components">,
  scopeAccounts: string,
  scopeConfig: string,
): ShortcutGroup[] {
  return [
    {
      title: tc("shortcuts.groupGlobal"),
      items: [
        { keys: ["⌘", "K"], label: tc("shortcuts.openPalette") },
        { keys: ["⌘", "/"], label: tc("shortcuts.showShortcuts") },
        { keys: ["⌘", "R"], label: tc("shortcuts.refreshSection") },
        {
          keys: ["⌘", "N"],
          label: tc("shortcuts.addAccount"),
          scope: scopeAccounts,
        },
        // ⌘F was listed here as "Focus filter (where exposed)" but no
        // section ever wired it — the hook option existed and nothing
        // passed it. Documenting a shortcut that does nothing is worse
        // than not documenting it.
        {
          keys: ["⌘", "⇧", "C"],
          label: tc("shortcuts.copyEmail"),
          scope: scopeAccounts,
        },
        { keys: ["⌘", "⇧", "L"], label: tc("shortcuts.focusLive") },
        // Bound in `useSidebarCollapsed` since it was added, and
        // surfaced only in the sidebar toggle's tooltip until now —
        // the inverse of the ⌘F problem noted above: a working
        // handler with no documentation.
        { keys: ["⌘", "\\"], label: tc("shortcuts.toggleSidebar") },
        // ⌘F focuses ConfigSection's content search. design.md said
        // "There is no ⌘F" — true when written, stale since a section
        // wired one. An undocumented working shortcut is the same
        // defect as a documented dead one, just harder to notice.
        {
          keys: ["⌘", "F"],
          label: tc("shortcuts.focusFilter"),
          scope: scopeConfig,
        },
      ],
    },
    {
      title: tc("shortcuts.groupModals"),
      items: [
        { keys: ["Esc"], label: tc("shortcuts.closeDialog") },
        { keys: ["Tab"], label: tc("shortcuts.cycleFocus") },
      ],
    },
    {
      title: tc("shortcuts.groupPalette"),
      items: [
        { keys: ["↑", "↓"], label: tc("shortcuts.moveSelection") },
        { keys: ["Home"], label: tc("shortcuts.firstResult") },
        { keys: ["End"], label: tc("shortcuts.lastResult") },
        { keys: ["Enter"], label: tc("shortcuts.runSelected") },
        { keys: ["Esc"], label: tc("shortcuts.closePalette") },
      ],
    },
    {
      title: tc("shortcuts.groupLiveStrip"),
      items: [
        { keys: ["j"], label: tc("shortcuts.nextSession") },
        { keys: ["k"], label: tc("shortcuts.prevSession") },
        { keys: ["Enter"], label: tc("shortcuts.openFocused") },
      ],
    },
  ];
}

/**
 * Global shortcut reference. Mounted at the shell level so it's
 * reachable from every section (⌘/ or the palette entry). Static
 * content — kept in sync with handlers by convention since shortcut
 * owners are spread across useSection, useGlobalShortcuts, App.tsx,
 * Modal, CommandPalette, and SidebarLiveStrip.
 */
export function ShortcutsModal({ onClose }: { onClose: () => void }) {
  const titleId = useId();
  // Two catalogs on purpose: `t` resolves section labels, which live
  // in the shell namespace next to the registry that owns them; `tc`
  // resolves this modal's own copy.
  const { t } = useTranslation("shell");
  const { t: tc } = useTranslation("components");
  // Live, not captured at import — see `navigationItems`.
  const enabled = useEnabledSections();
  const groups: ShortcutGroup[] = [
    {
      title: tc("shortcuts.groupNavigation"),
      items: navigationItems(
        enabled.map((s) => ({ id: s.id, label: t(s.labelKey) })),
        tc,
      ),
    },
    ...otherGroups(tc, t("sections.accounts"), t("sections.global")),
  ];
  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={titleId}>
      <ModalHeader
        title={tc("shortcuts.title")}
        id={titleId}
        onClose={onClose}
      />
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
          {tc("shortcuts.footer")}
        </p>
      </ModalBody>
    </Modal>
  );
}
