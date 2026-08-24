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

import { toMermaidColor } from './color.js';
import { sanitizeSvg } from './sanitizeSvg.js';

/**
 * Paper-mono colours for mermaid, read from the live tokens.
 *
 * Read at render rather than hardcoded so a diagram matches the theme it
 * is sitting in — including a switch while it is on screen.
 */
function themeVariables() {
  const cs = getComputedStyle(document.documentElement);
  // Every token here is `oklch()`, and mermaid runs each one through
  // khroma, which throws `Unsupported color format` on it — taking the
  // whole diagram down before layout ever starts. See `color.js`.
  const v = (name, fallback = '') => toMermaidColor(cs.getPropertyValue(name).trim() || fallback);
  // Sizes and families are NOT colours — `v` runs its value through
  // `toMermaidColor`, which is right for a paint and meaningless for a
  // length. `--f-sans` was going through it too.
  const raw = (name, fallback) => cs.getPropertyValue(name).trim() || fallback;
  const surface = v('--sf2');
  // `--sf` is the CARD colour and in light mode it is pure white
  // (`oklch(100% 0 0)`). That is right for a card floating on the warm
  // page; inside a diagram it painted every subgraph a stark white box
  // on the `--sf2` container, which read as a rendering fault rather
  // than as depth. `--bg` is the warm paper the whole app sits on — a
  // couple of percent off the container either way, so a cluster reads
  // as a panel instead of a hole.
  const panel = v('--bg');
  const line = v('--hair');
  const ink = v('--fg');
  const muted = v('--fg3');
  return {
    background: 'transparent',
    mainBkg: surface,
    secondBkg: v('--sf3'),
    tertiaryColor: panel,
    primaryColor: surface,
    primaryTextColor: ink,
    primaryBorderColor: v('--ac'),
    lineColor: muted,
    textColor: ink,
    titleColor: ink,
    edgeLabelBackground: v('--bg'),
    clusterBkg: panel,
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
    fontFamily: raw('--f-sans', 'system-ui'),
    // From the token: mermaid MEASURES text to size its nodes, so a
    // literal here mis-sizes every box the moment the panel's type
    // scale moves.
    fontSize: raw('--t-meta', '13px'),
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
          // **Pure SVG labels, never HTML ones.** By default mermaid
          // puts labels in a `<foreignObject>` containing HTML, and
          // that HTML contains unclosed `<br>`. The SVG it returns is
          // therefore HTML-flavoured, not well-formed XML — and the
          // strict parse below rejected it with "Opening and ending tag
          // mismatch: br line 1 and p". Measured: of three real
          // diagrams, the flowchart and the state diagram failed and
          // the sequence diagram drew, because only the first two
          // carried `<br/>` labels.
          //
          // Loosening the parser to `text/html` would have worked too
          // and is the wrong trade: the strict parse is a deliberate
          // guard, and the desktop renderer additionally STRIPS
          // `foreignObject` outright, so HTML labels would render there
          // as missing text rather than as an error. Turning them off
          // fixes both surfaces at the source, and `<br/>` still breaks
          // a line — mermaid emits tspans for it.
          htmlLabels: false,
          flowchart: { htmlLabels: false },
          class: { htmlLabels: false },
          themeVariables: themeVariables(),
        });
        const { svg } = await mermaid.render(id, source);
        if (cancelled || !ref.current) return;
        // `mermaid.render` returns sanitised SVG under `strict`. It is
        // parsed rather than assigned so nothing reaches an HTML parser
        // that would run a script if one were ever present.
        const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
        const el = doc.documentElement;
        if (el.nodeName.toLowerCase() !== 'svg') {
          throw new Error(`not an svg: ${(el.textContent || '').trim().replace(/\s+/g," ").slice(0, 400)}`);
        }
        // Scrubbed on the way in, exactly as the desktop renderer does.
        // Trusting mermaid's own `strict` mode is trusting one library's
        // sanitiser as the only barrier for input that is model output;
        // the desktop surface never did, and having one of two renderers
        // guarded is the same defect the htmlLabels fix already found
        // here.
        sanitizeSvg(el);
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
        {/* Say WHY. The reason was captured and then thrown away, so
            every failure looked identical from the outside — which is
            how one cause got fixed while a second went on producing the
            same sentence. It is small and muted: a reader who does not
            care skips it, and one who does has something to report. */}
        <p className="mmd-failed-note">
          Diagram could not be drawn — showing its source.
          {error && typeof error === 'string' ? ` (${error})` : ''}
        </p>
        <pre className="mono">{source}</pre>
      </div>
    );
  }

  return <div className="mmd" ref={ref} role="img" aria-label="diagram" />;
}
