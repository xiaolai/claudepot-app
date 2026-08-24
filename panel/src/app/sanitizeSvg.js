// The SVG scrubber, in a plain `.js` module rather than beside the
// component: `node --test` cannot load `.jsx`, and a security guard
// nobody can unit-test is the shape of guard that quietly stops
// working. Same reason `mdConfig.js` is not `mdConfig.jsx`.
/**
 * Strip the SVG-side script surfaces.
 *
 * Twin of `sanitizeSvg` in `src/sections/config/MermaidBlock.tsx`;
 * change one, change the other. Both are a second barrier behind
 * mermaid's `securityLevel: 'strict'` rather than a replacement for it.
 *
 * `foreignObject` is removed as well as `script`: it is the one SVG
 * element whose children are parsed as HTML.
 */
const DISALLOWED_SVG_TAGS = new Set(['script', 'foreignobject']);

export function sanitizeSvg(root) {
  // `querySelectorAll` skips the root, so scrub it explicitly first.
  scrubAttributes(root);
  // Collect before mutating; removals invalidate a live iteration.
  for (const el of Array.from(root.querySelectorAll('*'))) {
    if (DISALLOWED_SVG_TAGS.has(el.tagName.toLowerCase())) {
      el.remove();
      continue;
    }
    scrubAttributes(el);
  }
}

function scrubAttributes(el) {
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    if (name.startsWith('on')) {
      el.removeAttribute(attr.name);
      continue;
    }
    if (
      (name === 'href' || name === 'xlink:href' || name.endsWith(':href')) &&
      /^\s*javascript:/i.test(attr.value)
    ) {
      el.removeAttribute(attr.name);
    }
  }
}
