"use client";

/**
 * Progressive-enhancement renderer for fenced ```mermaid``` blocks.
 *
 * Mount once per page (typically on the post detail page). On hydrate
 * it walks the DOM under .proto-text and .proto-comment-body for
 * <pre><code class="language-mermaid">...</code></pre> blocks and
 * replaces each with an inline SVG produced by mermaid.render().
 *
 * On click of any rendered diagram, opens a fullscreen zoom modal
 * (GitHub-style): mouse-wheel zoom, drag-to-pan, toolbar buttons for
 * −/Reset/+, ESC or backdrop-click to close.
 *
 * Strict securityLevel — labels, links, and click bindings inside the
 * diagram source can't execute arbitrary JS. The diagram source is
 * already constrained by the markdown sanitizer (only language-*
 * classes survive), but mermaid's own parsing does interpret rich
 * syntax, so we belt-and-suspender by leaving its strictest mode on.
 *
 * The mermaid bundle (~250 KB) is loaded via dynamic `import()` so
 * pages without a diagram don't pay for it.
 */

import { useEffect } from "react";

const SELECTOR =
  ".proto-text pre code.language-mermaid, .proto-comment-body pre code.language-mermaid";

const ZOOM_MIN = 0.25;
const ZOOM_MAX = 8;
const ZOOM_STEP = 1.2;

const ICON_MINUS =
  '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="5" y1="12" x2="19" y2="12"/></svg>';
const ICON_PLUS =
  '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>';
const ICON_RESET =
  '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/></svg>';
const ICON_CLOSE =
  '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';

function openZoomModal(sourceWrap: HTMLElement) {
  const sourceSvg = sourceWrap.querySelector("svg");
  if (!sourceSvg) return;

  const overlay = document.createElement("div");
  overlay.className = "proto-mermaid-modal";
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", "Mermaid diagram, zoom view");

  const toolbar = document.createElement("div");
  toolbar.className = "proto-mermaid-modal-toolbar";

  const btnMinus = makeToolbarButton("Zoom out", ICON_MINUS);
  const btnReset = makeToolbarButton("Reset zoom", ICON_RESET);
  const btnPlus = makeToolbarButton("Zoom in", ICON_PLUS);
  const btnClose = makeToolbarButton("Close zoom view", ICON_CLOSE);
  toolbar.append(btnMinus, btnReset, btnPlus, btnClose);

  const pane = document.createElement("div");
  pane.className = "proto-mermaid-modal-pane";

  const svg = sourceSvg.cloneNode(true) as SVGSVGElement;
  // Strip mermaid's max-width style so the diagram is free to zoom up.
  svg.removeAttribute("style");
  svg.removeAttribute("width");
  svg.removeAttribute("height");
  svg.style.transformOrigin = "center center";
  svg.style.transition = "transform 0.08s ease-out";
  pane.appendChild(svg);

  overlay.append(toolbar, pane);
  document.body.appendChild(overlay);

  // Lock page scroll while the modal is open.
  const prevOverflow = document.body.style.overflow;
  document.body.style.overflow = "hidden";

  // Pan + zoom state.
  let scale = 1;
  let tx = 0;
  let ty = 0;
  let dragging = false;
  let dragStartX = 0;
  let dragStartY = 0;
  let dragOriginX = 0;
  let dragOriginY = 0;

  function apply() {
    svg.style.transform = `translate(${tx}px, ${ty}px) scale(${scale})`;
  }

  function setScale(next: number) {
    scale = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, next));
    apply();
  }

  function reset() {
    scale = 1;
    tx = 0;
    ty = 0;
    apply();
  }

  function close() {
    document.body.style.overflow = prevOverflow;
    document.removeEventListener("keydown", onKey);
    overlay.remove();
    // Restore focus to the diagram in the page so keyboard users
    // don't lose context.
    sourceWrap.focus({ preventScroll: false });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") close();
    else if (e.key === "+" || e.key === "=") setScale(scale * ZOOM_STEP);
    else if (e.key === "-" || e.key === "_") setScale(scale / ZOOM_STEP);
    else if (e.key === "0") reset();
  }

  btnMinus.addEventListener("click", () => setScale(scale / ZOOM_STEP));
  btnPlus.addEventListener("click", () => setScale(scale * ZOOM_STEP));
  btnReset.addEventListener("click", reset);
  btnClose.addEventListener("click", close);

  pane.addEventListener("wheel", (e) => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 1 / ZOOM_STEP : ZOOM_STEP;
    setScale(scale * factor);
  }, { passive: false });

  pane.addEventListener("pointerdown", (e) => {
    dragging = true;
    pane.setPointerCapture(e.pointerId);
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    dragOriginX = tx;
    dragOriginY = ty;
  });
  pane.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    tx = dragOriginX + (e.clientX - dragStartX);
    ty = dragOriginY + (e.clientY - dragStartY);
    apply();
  });
  pane.addEventListener("pointerup", () => { dragging = false; });
  pane.addEventListener("pointercancel", () => { dragging = false; });

  // Click on the backdrop (overlay) closes; click on toolbar or pane
  // does not (those are children, e.target won't be the overlay).
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });

  document.addEventListener("keydown", onKey);

  // Focus the close button so ESC + Tab work immediately.
  btnClose.focus();
}

function makeToolbarButton(label: string, iconHtml: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "proto-mermaid-modal-btn";
  b.setAttribute("aria-label", label);
  b.title = label;
  b.innerHTML = iconHtml;
  return b;
}

export function MermaidEnhancer() {
  useEffect(() => {
    const blocks = Array.from(
      document.querySelectorAll<HTMLElement>(SELECTOR),
    );
    if (blocks.length === 0) return;

    let cancelled = false;
    const cleanups: Array<() => void> = [];

    (async () => {
      const mod = await import("mermaid");
      const mermaid = mod.default ?? mod;
      if (cancelled) return;

      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "neutral",
      });

      for (const code of blocks) {
        if (cancelled) return;
        const source = code.textContent ?? "";
        const id = `mmd-${Math.random().toString(36).slice(2, 10)}`;
        try {
          const { svg } = await mermaid.render(id, source);
          const wrap = document.createElement("div");
          wrap.className = "proto-mermaid";
          wrap.innerHTML = svg;
          // A11y: make the diagram a real focus target so keyboard
          // users can open the zoom modal via Enter / Space.
          wrap.setAttribute("role", "button");
          wrap.setAttribute("tabindex", "0");
          wrap.setAttribute("aria-label", "Open diagram in zoom view");
          const onClick = () => openZoomModal(wrap);
          const onKey = (e: KeyboardEvent) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              openZoomModal(wrap);
            }
          };
          wrap.addEventListener("click", onClick);
          wrap.addEventListener("keydown", onKey);
          cleanups.push(() => {
            wrap.removeEventListener("click", onClick);
            wrap.removeEventListener("keydown", onKey);
          });
          const pre = code.parentElement;
          if (pre && pre.parentElement) {
            pre.parentElement.replaceChild(wrap, pre);
          }
        } catch (err) {
          // Leave the source code block in place if rendering fails —
          // a broken diagram is better than empty space, and surfaces
          // the parse error to the author.
          console.warn("[mermaid] render failed:", err);
        }
      }
    })();

    return () => {
      cancelled = true;
      for (const fn of cleanups) fn();
    };
  }, []);

  return null;
}
