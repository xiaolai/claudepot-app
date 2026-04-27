import { useEffect, useId, useRef, useState } from "react";

/**
 * Lazy-rendered Mermaid diagram for markdown ` ```mermaid ` fences.
 *
 * Loading: the `mermaid` library (~200 KB gzipped) is imported via
 * dynamic `import("mermaid")` only when this component first mounts,
 * so users who never preview a diagram-bearing file don't pay the
 * parse cost. Vite splits the dynamic import into its own chunk.
 *
 * Theming: paper-mono colors are read from CSS custom properties at
 * render time and passed to mermaid's `themeVariables` API. We
 * subscribe to `data-theme` changes on `<html>` so a theme toggle
 * triggers a re-render with fresh colors.
 *
 * SECURITY: `securityLevel: "strict"` disables embedded HTML, click
 * handlers, and arbitrary script in mermaid source. The SVG that
 * comes back is sanitized by mermaid before we inject it. Source
 * has already passed through `claudepot_core::config_view::mask`.
 */
export function MermaidBlock({ source }: { source: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const reactId = useId();
  const id = `mermaid-${reactId.replace(/[:]/g, "-")}`;
  const [error, setError] = useState<string | null>(null);
  const [themeVersion, setThemeVersion] = useState(0);

  // Re-render on theme change (paper-mono toggles `[data-theme]` on
  // <html>; the previous SVG was painted with the wrong palette).
  useEffect(() => {
    const obs = new MutationObserver(() => setThemeVersion((v) => v + 1));
    obs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    let cancelled = false;
    setError(null);

    // Clear the previous render synchronously so a theme toggle or
    // source change never shows a stale-colored diagram while the
    // async re-render is in flight.
    if (ref.current) ref.current.replaceChildren();

    (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        if (cancelled) return;

        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "base",
          themeVariables: readThemeVariables(),
          fontFamily:
            getComputedStyle(document.documentElement)
              .getPropertyValue("--font")
              .trim() || "monospace",
        });

        const { svg } = await mermaid.render(id, source);
        if (cancelled || !ref.current) return;
        attachSvg(ref.current, svg);
      } catch (err) {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [source, id, themeVersion]);

  if (error) {
    return (
      <div className="mermaid-error" role="alert">
        <div className="mermaid-error-title">Mermaid render failed</div>
        <pre className="mermaid-error-msg">{error}</pre>
        <pre className="mermaid-error-source">{source}</pre>
      </div>
    );
  }

  return (
    <div
      ref={ref}
      className="mermaid-block"
      role="img"
      aria-label="Mermaid diagram"
    />
  );
}

// ---------- SVG attach (no innerHTML) --------------------------------

/**
 * Parse mermaid's serialized SVG with DOMParser, sanitize it, and
 * adopt only the top-level `<svg>` subtree into the container.
 * Defense-in-depth on top of mermaid's `securityLevel: "strict"`:
 *   - We never set `innerHTML` on a live DOM node.
 *   - We reject anything whose root isn't `<svg>` in the SVG
 *     namespace — if mermaid ever returns an error doc or non-SVG
 *     markup, we surface it as an error instead of mounting it.
 *   - We strip `<script>` and `<foreignObject>` descendants, every
 *     `on*` event-handler attribute, and any `href` / `xlink:href`
 *     pointing at `javascript:` URIs. This kills the well-known
 *     SVG-side XSS surfaces a sanitizer bypass would otherwise
 *     reach.
 *
 * On parse failure, throw — the caller catches and renders the
 * error UI with the diagram source.
 */
function attachSvg(container: HTMLElement, svgString: string): void {
  const doc = new DOMParser().parseFromString(svgString, "image/svg+xml");
  const parserError = doc.querySelector("parsererror");
  if (parserError) {
    throw new Error(parserError.textContent ?? "SVG parse error");
  }
  const root = doc.documentElement;
  if (
    !root ||
    root.namespaceURI !== "http://www.w3.org/2000/svg" ||
    root.tagName.toLowerCase() !== "svg"
  ) {
    throw new Error("Mermaid returned non-SVG markup");
  }
  sanitizeSvg(root);
  container.replaceChildren(document.importNode(root, true));
}

const DISALLOWED_SVG_TAGS = new Set(["script", "foreignobject"]);

function sanitizeSvg(root: Element): void {
  // Same attribute-scrub policy applies to the root and every
  // descendant; querySelectorAll skips the root, so we run it
  // explicitly first.
  scrubAttributes(root);
  // Walk depth-first via querySelectorAll — collect first, then
  // mutate, so removals don't invalidate iteration.
  for (const el of Array.from(root.querySelectorAll("*"))) {
    if (DISALLOWED_SVG_TAGS.has(el.tagName.toLowerCase())) {
      el.remove();
      continue;
    }
    scrubAttributes(el);
  }
}

function scrubAttributes(el: Element): void {
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    // Inline event handlers (`onclick`, `onload`, …).
    if (name.startsWith("on")) {
      el.removeAttribute(attr.name);
      continue;
    }
    // SVG `<a>` link vectors. mermaid in strict mode shouldn't
    // emit these, but an upstream regression that did would be
    // a one-click XSS.
    if (
      (name === "href" || name === "xlink:href" || name.endsWith(":href")) &&
      /^\s*javascript:/i.test(attr.value)
    ) {
      el.removeAttribute(attr.name);
    }
  }
}

// ---------- Theme bridge ---------------------------------------------

/**
 * Read paper-mono custom properties off `<html>` and map them to
 * mermaid's theme-variable names. Mermaid accepts arbitrary CSS color
 * strings (oklch, hex, rgb), so we hand the literals through. If a
 * variable is missing, mermaid's "base" theme falls back gracefully.
 */
function readThemeVariables(): Record<string, string> {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return {
    // Surfaces
    background: v("--bg") || "transparent",
    mainBkg: v("--bg-sunken"),
    secondBkg: v("--bg-elev"),
    tertiaryColor: v("--bg-hover"),
    // Primary node
    primaryColor: v("--bg-sunken"),
    primaryTextColor: v("--fg"),
    primaryBorderColor: v("--line-strong"),
    // Lines + labels
    lineColor: v("--fg-muted"),
    textColor: v("--fg"),
    titleColor: v("--fg"),
    edgeLabelBackground: v("--bg"),
    // Cluster (subgraphs)
    clusterBkg: v("--bg-sunken"),
    clusterBorder: v("--line"),
    // Notes
    noteBkgColor: v("--bg-sunken"),
    noteTextColor: v("--fg"),
    noteBorderColor: v("--line"),
    // Sequence diagrams
    actorBkg: v("--bg-sunken"),
    actorBorder: v("--line-strong"),
    actorTextColor: v("--fg"),
    actorLineColor: v("--fg-muted"),
    signalColor: v("--fg-muted"),
    signalTextColor: v("--fg"),
    labelBoxBkgColor: v("--bg-sunken"),
    labelBoxBorderColor: v("--line"),
    labelTextColor: v("--fg"),
    loopTextColor: v("--fg"),
    activationBkgColor: v("--bg-elev"),
    activationBorderColor: v("--line"),
    sequenceNumberColor: v("--fg-muted"),
    // Gantt
    sectionBkgColor: v("--bg-sunken"),
    altSectionBkgColor: v("--bg-elev"),
    gridColor: v("--line"),
    todayLineColor: v("--accent"),
    taskBkgColor: v("--accent-soft"),
    taskBorderColor: v("--accent-border"),
    taskTextColor: v("--fg"),
    // Misc
    fontSize: "13px",
  };
}
