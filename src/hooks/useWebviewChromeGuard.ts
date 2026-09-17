import { useEffect } from "react";
import { isEditable } from "./useGlobalShortcuts";

/**
 * A packaged build is not a browser tab.
 *
 * The webview under this app still carries a browser's reload
 * affordances, and they mean nothing here: there is no address bar,
 * no tab to restore, and nothing on screen says the window can be
 * thrown away. What a reload does do is discard everything that lives
 * only in the renderer — an open modal, a half-typed secret in an Add
 * dialog, the transcript you had scrolled to, an op-progress modal's
 * event subscription.
 *
 * Two affordances, at two layers:
 *
 * - **The reload keys.** ⌘R is a documented app shortcut (refresh the
 *   section) and only Accounts and Projects pass a handler for it, so
 *   on the other eight sections it falls through to the webview. On
 *   macOS that appears to do nothing — wry's `performKeyEquivalent`
 *   hands the key to the app menu and `app_menu.rs` deliberately
 *   binds no accelerator there, which is read from that key path and
 *   not measured — but on WebView2 F5 /
 *   Ctrl+R / Ctrl+Shift+R are "browser accelerator keys" and are on
 *   by default. wry can turn those off
 *   (`with_browser_accelerator_keys`); Tauri 2.11 does not expose it,
 *   so cancelling the keydown is the layer available from here.
 * - **The webview's own context menu**, which carries the browser's
 *   Reload — WebView2's and webkit2gtk's default menus do; WKWebView's
 *   was not measured here. It is suppressed for targets that are not
 *   text; an editable field and a live selection keep theirs, because
 *   Copy / Paste / Look Up are what that menu is for in an app.
 *
 * Suppression, not a shortcut — so unlike every key in
 * `useShellShortcuts` it is deliberately NOT behind
 * `isShortcutContextBlocked()`. A field with focus is the state where
 * a reload costs the most, and cancelling a keystroke the webview
 * would have eaten takes nothing away from the person typing.
 *
 * It does not touch `location.reload()`: the ErrorBoundary's Reload
 * button is the sanctioned way out of a crashed renderer, and
 * guarding the *input* is what leaves that path alone. That is also
 * why this lives in the renderer rather than in Tauri's navigation
 * handler — the handler sees a reload and cannot tell a keypress from
 * the app's own decision.
 *
 * Off in dev, where reload and Inspect Element are the loop.
 */

type Combo = Pick<
  KeyboardEvent,
  "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
>;

/**
 * True for the key combinations a webview reads as "reload this
 * page": F5 and its Ctrl/Shift hard-reload variants, ⌘R / ⌃R, and
 * ⌘⇧R / ⌃⇧R. Shift makes `e.key` report "R", so the letter is
 * matched case-insensitively (same convention as ⌘⇧L). ⌥ excludes the
 * combo — ⌥⌘R is nobody's reload, and swallowing it would take a
 * modifier chord away from a future shortcut for no gain.
 */
export function isReloadCombo(e: Combo): boolean {
  if (e.key === "F5") return true;
  if (e.key.toLowerCase() !== "r") return false;
  if (e.altKey) return false;
  return e.metaKey || e.ctrlKey;
}

/**
 * True when the native context menu is the one the user wants: a
 * right-click in an editable field, or on the element holding a live
 * selection. Everything else is app chrome, where the native menu
 * offers only webview verbs.
 *
 * The selection has to be *at* the click — `target` equal to or
 * containing the anchor node. A selection made in another pane leaves
 * `isCollapsed` false for as long as it stands, so testing that alone
 * would hand the native menu back for every right-click afterwards.
 */
export function nativeContextMenuWarranted(
  target: Element | null,
  selection: Selection | null,
): boolean {
  if (isEditable(target)) return true;
  if (!selection || selection.isCollapsed || !target) return false;
  const anchor = selection.anchorNode;
  return !!anchor && (target === anchor || target.contains(anchor));
}

/**
 * Install both guards. `enabled` defaults to "this is not a dev
 * build" and is a parameter so the behaviour is testable in both
 * directions — a guard nobody has watched stay out of the way is as
 * untrustworthy as one nobody has watched fire.
 */
export function useWebviewChromeGuard(
  enabled: boolean = !import.meta.env.DEV,
): void {
  useEffect(() => {
    if (!enabled) return;
    // Capture phase: this has to see the key even when a modal or a
    // palette input has called stopPropagation on its own keydown.
    // It only ever calls preventDefault, never stopPropagation, so
    // ⌘R still reaches the section handler that refreshes the list.
    const onKey = (e: KeyboardEvent) => {
      if (isReloadCombo(e)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    // Bubble phase on `document`, so React's own root-level handlers
    // run first: every app context menu (account card, project row,
    // session row) already calls preventDefault, and this stays out
    // of the way of one that has.
    const onMenu = (e: MouseEvent) => {
      if (e.defaultPrevented) return;
      if (
        nativeContextMenuWarranted(
          e.target as Element | null,
          window.getSelection(),
        )
      ) {
        return;
      }
      e.preventDefault();
    };
    document.addEventListener("contextmenu", onMenu);
    return () => document.removeEventListener("contextmenu", onMenu);
  }, [enabled]);
}
