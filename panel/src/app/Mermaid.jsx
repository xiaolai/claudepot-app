// A mermaid diagram in a transcript.
//
// A diagram in an answer *is* the answer; showing its source is showing
// the wrong artifact. This is the panel's counterpart to the desktop's
// `sections/config/MermaidBlock` — same posture, different constraints.
//
// ## Loading
//
// `import('mermaid')` runs on first mount, never at page load. That
// matters far more here than on the desktop: mermaid and its diagram
// packs are **larger than the rest of this bundle put together**, the
// panel is served `no-store`, and the client is a phone on a LAN. A
// thread with no diagram in it must not pay for one, and does not.
//
// Vite splits the dynamic import — and mermaid's own per-diagram
// imports — into separate chunks, so a flowchart pulls the flowchart
// pack and not the cytoscape or katex ones. `remote::assets` serves
// them from `/panel/chunks/`; see `scripts/build-panel.sh` for how that
// route table is generated and kept honest.
//
// ## Security
//
// `securityLevel: 'strict'` disables embedded HTML, click handlers and
// script in diagram source — which matters, because the source here is
// model output that may quote a file. Mermaid sanitises the SVG it
// returns before it is inserted.
//
// It also runs under the panel's CSP unchanged: `script-src 'self'` with
// no `unsafe-eval`. Verified against the built chunks — mermaid's core,
// its diagram packs and cytoscape contain no `eval` and no
// `new Function`. Inline styles it emits are already covered by
// `style-src 'unsafe-inline'`, which the design system requires anyway.
import { useEffect, useId, useRef, useState } from 'react';

/**
 * Paper-mono colours for mermaid, read from the live tokens.
 *
 * Read at render rather than hardcoded so a diagram matches the theme it
 * is sitting in — including a switch while it is on screen.
 */
function themeVariables() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name, fallback = '') => cs.getPropertyValue(name).trim() || fallback;
  const surface = v('--sf2');
  const line = v('--hair');
  const ink = v('--fg');
  const muted = v('--fg3');
  return {
    background: 'transparent',
    mainBkg: surface,
    secondBkg: v('--sf3'),
    tertiaryColor: v('--sf'),
    primaryColor: surface,
    primaryTextColor: ink,
    primaryBorderColor: v('--ac'),
    lineColor: muted,
    textColor: ink,
    titleColor: ink,
    edgeLabelBackground: v('--bg'),
    clusterBkg: v('--sf'),
    clusterBorder: line,
    noteBkgColor: surface,
    noteTextColor: ink,
    noteBorderColor: line,
    actorBkg: surface,
    actorBorder: v('--ac'),
    actorTextColor: ink,
    actorLineColor: muted,
    signalColor: muted,
    signalTextColor: ink,
    // Mermaid measures text to lay out nodes, so it has to be told the
    // family the panel actually paints with or every box is mis-sized.
    fontFamily: v('--f-sans', 'system-ui'),
    fontSize: '13px',
  };
}

export function Mermaid({ source }) {
  const ref = useRef(null);
  const reactId = useId();
  const id = `mmd-${reactId.replace(/:/g, '-')}`;
  const [error, setError] = useState(null);
  const [themeVersion, setThemeVersion] = useState(0);

  // The shell writes `data-t` on a theme change. A diagram painted with
  // the old palette would otherwise sit there in the wrong colours.
  useEffect(() => {
    const el = document.querySelector('.panel');
    if (!el) return undefined;
    const obs = new MutationObserver(() => setThemeVersion((v) => v + 1));
    obs.observe(el, { attributes: true, attributeFilter: ['data-t'] });
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    // Clear synchronously: a stale diagram in the old palette is worse
    // than an empty box for the moment the re-render takes.
    if (ref.current) ref.current.replaceChildren();

    (async () => {
      try {
        const mermaid = (await import('mermaid')).default;
        if (cancelled) return;
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: 'base',
          themeVariables: themeVariables(),
        });
        const { svg } = await mermaid.render(id, source);
        if (cancelled || !ref.current) return;
        // `mermaid.render` returns sanitised SVG under `strict`. It is
        // parsed rather than assigned so nothing reaches an HTML parser
        // that would run a script if one were ever present.
        const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
        const el = doc.documentElement;
        if (el.nodeName.toLowerCase() !== 'svg') throw new Error('not an svg');
        ref.current.replaceChildren(document.importNode(el, true));
      } catch (e) {
        if (cancelled) return;
        // A diagram the model got slightly wrong is common, and it must
        // not take the thread down with it. The source is shown instead,
        // which is what the reader had before this component existed.
        setError(e instanceof Error ? e.message : 'could not render');
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [id, source, themeVersion]);

  if (error) {
    return (
      <div className="mmd-failed">
        <p className="mmd-failed-note">Diagram could not be drawn — showing its source.</p>
        <pre className="mono">{source}</pre>
      </div>
    );
  }

  return <div className="mmd" ref={ref} role="img" aria-label="diagram" />;
}
