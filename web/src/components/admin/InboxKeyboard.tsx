"use client";

import { useEffect, useRef, useState } from "react";

/**
 * Keyboard navigation for the Today inbox.
 *
 * Bindings (only fire when no input is focused and no modal is open):
 *   j / ArrowDown  — next row
 *   k / ArrowUp    — previous row
 *   a              — primary action (Approve / Dismiss / Uphold)
 *   r              — destructive action (Reject / Remove / Restore)
 *   ?              — toggle cheat sheet
 *   Esc            — close cheat sheet
 *
 * Row targets are `[data-inbox-row]` elements within
 * `[data-inbox-stream]`. Action buttons are matched by class —
 * `.proto-mod-btn-keep` for primary, `.proto-mod-btn-remove` for
 * destructive — which is also how the row markup is authored in
 * InboxRow.tsx. Keep both in sync.
 */
export function InboxKeyboard() {
  const [activeIdx, setActiveIdx] = useState<number | null>(null);
  const [cheatOpen, setCheatOpen] = useState(false);
  const activeIdxRef = useRef<number | null>(activeIdx);
  activeIdxRef.current = activeIdx;

  useEffect(() => {
    function rows(): HTMLElement[] {
      return Array.from(
        document.querySelectorAll<HTMLElement>("[data-inbox-row]"),
      );
    }
    function setActive(idx: number) {
      const all = rows();
      all.forEach((r, i) =>
        r.toggleAttribute("data-inbox-row-active", i === idx),
      );
      const target = all[idx];
      if (target)
        target.scrollIntoView({ block: "nearest", behavior: "smooth" });
      setActiveIdx(idx);
    }
    function clickInRow(
      idx: number | null,
      selector: string,
    ) {
      if (idx === null) return;
      const row = rows()[idx];
      if (!row) return;
      const btn = row.querySelector<HTMLButtonElement>(selector);
      if (btn) btn.click();
    }
    function isTypingTarget(el: EventTarget | null): boolean {
      if (!(el instanceof HTMLElement)) return false;
      const tag = el.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT")
        return true;
      if (el.isContentEditable) return true;
      return false;
    }

    function onKey(e: KeyboardEvent) {
      if (isTypingTarget(e.target)) return;
      // Don't fire while a Modal is open. Modals carry role=dialog
      // per the design rules; if anything in the page has it,
      // assume modal.
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;

      const all = rows();
      if (all.length === 0 && e.key !== "?") return;

      switch (e.key) {
        case "j":
        case "ArrowDown": {
          e.preventDefault();
          const next = Math.min(
            (activeIdxRef.current ?? -1) + 1,
            all.length - 1,
          );
          setActive(Math.max(0, next));
          break;
        }
        case "k":
        case "ArrowUp": {
          e.preventDefault();
          const prev = Math.max((activeIdxRef.current ?? 1) - 1, 0);
          setActive(prev);
          break;
        }
        case "a":
          e.preventDefault();
          clickInRow(activeIdxRef.current, ".proto-mod-btn-keep");
          break;
        case "r":
          e.preventDefault();
          clickInRow(activeIdxRef.current, ".proto-mod-btn-remove");
          break;
        case "?":
          e.preventDefault();
          setCheatOpen((v) => !v);
          break;
        case "Escape":
          if (cheatOpen) {
            e.preventDefault();
            setCheatOpen(false);
          }
          break;
      }
    }

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cheatOpen]);

  if (!cheatOpen) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Inbox keyboard shortcuts"
      className="proto-inbox-cheat"
      onClick={(e) => {
        if (e.target === e.currentTarget) setCheatOpen(false);
      }}
    >
      <div className="proto-inbox-cheat-panel">
        <h2>Keyboard shortcuts</h2>
        <dl>
          <dt>
            <kbd>j</kbd> / <kbd>↓</kbd>
          </dt>
          <dd>next row</dd>
          <dt>
            <kbd>k</kbd> / <kbd>↑</kbd>
          </dt>
          <dd>previous row</dd>
          <dt>
            <kbd>a</kbd>
          </dt>
          <dd>primary action (approve / dismiss / uphold)</dd>
          <dt>
            <kbd>r</kbd>
          </dt>
          <dd>destructive action (reject / remove / restore)</dd>
          <dt>
            <kbd>?</kbd>
          </dt>
          <dd>toggle this cheat sheet</dd>
          <dt>
            <kbd>Esc</kbd>
          </dt>
          <dd>close</dd>
        </dl>
        <button
          type="button"
          className="proto-btn"
          onClick={() => setCheatOpen(false)}
        >
          Close
        </button>
      </div>
    </div>
  );
}
