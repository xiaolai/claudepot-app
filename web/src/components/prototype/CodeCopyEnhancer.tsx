"use client";

/**
 * Wires the SSR'd `.proto-code-copy` button shells to the clipboard.
 *
 * The button itself, the language label, and the line gutter are all
 * baked into the HTML by decorateCodeBlocks() in src/lib/markdown.ts —
 * nothing here paints. We just attach a click handler that reads the
 * code lines (via .textContent so HTML-escaped characters decode
 * naturally), writes to navigator.clipboard, and toggles the icon to
 * a check + "Copied" label for ~1.5s before reverting.
 *
 * Mount once per page (typically on the post detail page). Idempotent
 * across re-mounts thanks to AbortController.
 */

import { useEffect } from "react";

const COPY_ICON =
  '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"></rect>' +
  '<path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"></path>' +
  "</svg>";

const CHECK_ICON =
  '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M20 6 9 17l-5-5"></path>' +
  "</svg>";

// The button is icon-only by design. aria-label and title carry the
// accessible name; the visible content is just the SVG.
const COPY_LABEL = COPY_ICON;
const COPIED_LABEL = CHECK_ICON;

const REVERT_AFTER_MS = 1500;

export function CodeCopyEnhancer() {
  useEffect(() => {
    const buttons = Array.from(
      document.querySelectorAll<HTMLButtonElement>(
        ".proto-code .proto-code-copy",
      ),
    );
    if (buttons.length === 0) return;

    const controller = new AbortController();
    const revertTimers = new WeakMap<HTMLButtonElement, ReturnType<typeof setTimeout>>();

    for (const btn of buttons) {
      btn.addEventListener(
        "click",
        async () => {
          const wrapper = btn.closest(".proto-code");
          if (!wrapper) return;
          const lines = wrapper.querySelectorAll<HTMLElement>(
            ".proto-code-line",
          );
          // textContent reads the displayed string with HTML entities
          // decoded — exactly what the user expects on paste. Empty
          // source lines were rendered with a single nbsp (U+00A0) so
          // they take vertical space; un-do that here so the paste
          // round-trips an empty line, not a phantom whitespace char.
          const text = Array.from(lines)
            .map((l) => {
              const raw = l.textContent ?? "";
              return raw === " " ? "" : raw;
            })
            .join("\n");
          try {
            await navigator.clipboard.writeText(text);
          } catch (err) {
            console.warn("[code-copy] clipboard write failed:", err);
            return;
          }
          btn.dataset.state = "copied";
          btn.innerHTML = COPIED_LABEL;
          const prev = revertTimers.get(btn);
          if (prev) clearTimeout(prev);
          const next = setTimeout(() => {
            if (!btn.isConnected) return;
            btn.dataset.state = "";
            btn.innerHTML = COPY_LABEL;
          }, REVERT_AFTER_MS);
          revertTimers.set(btn, next);
        },
        { signal: controller.signal },
      );
    }

    return () => controller.abort();
  }, []);

  return null;
}
