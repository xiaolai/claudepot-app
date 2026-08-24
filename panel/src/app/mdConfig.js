// The markdown pipeline's configuration, separated from the component.
//
// Not a style choice: this file is plain JS with `createElement` rather
// than JSX so that `node --test` can import it directly. The panel has
// no JSX-capable test runner, and the alternative was a security posture
// asserted only in a comment. Everything here that matters — no raw
// HTML, images that are not images, links that cannot navigate the panel
// away — is exercised against real rendered output in `markdown.test.js`.
//
// `Markdown.jsx` is the thin JSX wrapper that consumes this. If you add
// a plugin, add it here, and the test that counts the plugins will make
// you say so out loud.
import { Children, createElement as h, isValidElement, lazy, Suspense } from 'react';
import remarkGfm from 'remark-gfm';

/**
 * The diagram renderer, behind `lazy`.
 *
 * Two things at once, and both are load-bearing. It defers ~3.4 MB of
 * mermaid until a diagram is actually on screen — and it keeps the
 * import path a *string inside a callback*, so this file stays parseable
 * by `node --test`. A plain `import … from './Mermaid.jsx'` broke that
 * immediately: node has no JSX loader, and the whole reason this module
 * is JSX-free is that its security assertions run there.
 */
/**
 * Is this `<code>` className tagged with `language-<name>`?
 *
 * `className.includes('language-mermaid')` is a substring test on a
 * space-delimited class list, so `language-mermaidish` matched and an
 * ordinary fence was handed to the diagram renderer. Compare whole
 * tokens.
 *
 * Twin of `src/lib/codeFence.ts` — the panel is a separate Vite app and
 * cannot import from the Tauri renderer. Change one, change the other;
 * both carry the `mermaidish` case in their tests.
 */
export function hasLanguage(className, language) {
  if (!className) return false;
  return className.split(/\s+/).includes(`language-${language}`);
}

/**
 * Protocols a link in model output may use.
 *
 * `react-markdown`'s default `urlTransform` blocks `javascript:` and
 * friends but permits relative URLs, `irc:`, `ircs:` and `xmpp:`. The
 * input is model output quoting arbitrary files, so it does not get to
 * pick a protocol handler.
 *
 * Matched lexically, with no `new URL()`: parsing needs a base, and a
 * base is a URL literal, which `remote::assets`'s "no third-party
 * requests" test correctly refuses to find in this bundle. Anything
 * that does not match the allowed prefix is refused, so the failure
 * direction is closed.
 *
 * Twin of `src/lib/mdLink.ts`; change one, change the other.
 */
const ALLOWED_SCHEME = /^(?:https?|mailto):/i;

export function isSafeHref(href) {
  if (!href) return false;
  return ALLOWED_SCHEME.test(String(href).trim());
}

const Mermaid = lazy(() => import('./Mermaid.jsx').then((m) => ({ default: m.Mermaid })));

/**
 * Remark plugins. GFM only — tables, strikethrough, task lists,
 * autolinks. That is what Claude writes.
 */
export const REMARK_PLUGINS = [remarkGfm];

/**
 * Rehype plugins: **none, deliberately.**
 *
 * Exported as an empty array rather than omitted so the test can assert
 * emptiness. The one that would be reached for is `rehype-raw`, which
 * turns embedded HTML in the source into real elements — on a surface
 * whose input is model output quoting arbitrary files. The second is
 * `rehype-highlight`, which the desktop app uses and this one skips
 * because highlight.js is larger than this whole bundle and the panel is
 * served `no-store`.
 */
export const REHYPE_PLUGINS = [];

/** Flatten a react-markdown `<code>` subtree back to its source text. */
function codeText(node) {
  if (typeof node === 'string') return node;
  if (typeof node === 'number') return String(node);
  if (node === null || node === undefined || typeof node === 'boolean') return '';
  if (Array.isArray(node)) return node.map(codeText).join('');
  if (isValidElement(node)) return codeText(node.props?.children);
  return '';
}

export const MD_COMPONENTS = {
  // `pre`, not `code`: a diagram replaces the whole block, and the
  // default `<pre>` would box the SVG in scrollbars and a code border.
  pre: ({ children, node: _node, ...rest }) => {
    const codeChild = Children.toArray(children).find(isValidElement);
    const className = codeChild?.props?.className ?? '';
    if (hasLanguage(className, 'mermaid')) {
      const source = codeText(codeChild?.props?.children);
      // The fallback is the diagram's source, which is exactly what the
      // reader had before this existed — so a slow chunk degrades to the
      // old behaviour rather than to a blank space.
      return h(Suspense, { fallback: h('pre', null, source) }, h(Mermaid, { source }));
    }
    return h('pre', rest, children);
  },

  // A transcript's images are not the panel's to fetch. The server's CSP
  // is `img-src 'self' data:`, so a remote one would be blocked and show
  // a broken icon; one that *did* load would be a tracking pixel. Alt
  // text is safer and says more about what is actually there.
  img: ({ alt, src }) =>
    h(
      'span',
      { className: 'md-img', title: typeof src === 'string' ? src : undefined },
      alt || 'image',
    ),

  // A tapped link must not navigate the panel away from itself — that
  // would drop an authenticated session mid-thread. react-markdown's
  // default `urlTransform` has already dropped anything outside
  // http/https/mailto/tel, so `javascript:` never reaches here.
  // Only http(s) and mailto become a real link. See `isSafeHref`.
  a: ({ href, children }) =>
    isSafeHref(href)
      ? h('a', { href, target: '_blank', rel: 'noopener noreferrer' }, children)
      : h('span', { title: href || undefined }, children),

  // A GFM table is the one construct that reliably exceeds a phone's
  // width. It scrolls inside its own wrapper so it cannot widen the
  // column and break every other row's layout.
  table: ({ children }) =>
    h('div', { className: 'md-table-scroll' }, h('table', null, children)),
};
