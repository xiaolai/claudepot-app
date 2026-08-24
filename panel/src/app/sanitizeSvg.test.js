import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { sanitizeSvg } from './sanitizeSvg.js';

/**
 * Twin of the desktop's `sanitizeSvg`. The panel used to mount
 * mermaid's output directly, trusting one library's `strict` mode as
 * the only barrier for what is ultimately model output.
 */
function parse(svg) {
  const dom = new JSDOM();
  const doc = new dom.window.DOMParser().parseFromString(svg, 'image/svg+xml');
  return doc.documentElement;
}

test('a script element is removed', () => {
  const el = parse('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><g/></svg>');
  sanitizeSvg(el);
  assert.equal(el.querySelector('script'), null);
  assert.ok(el.querySelector('g'));
});

test('a foreignObject is removed — its children are parsed as HTML', () => {
  const el = parse(
    '<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><b>x</b></foreignObject></svg>',
  );
  sanitizeSvg(el);
  assert.equal(el.querySelector('foreignObject'), null);
});

test('inline event handlers are stripped, on the root too', () => {
  const el = parse(
    '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><rect onclick="alert(2)"/></svg>',
  );
  sanitizeSvg(el);
  assert.equal(el.getAttribute('onload'), null);
  assert.equal(el.querySelector('rect').getAttribute('onclick'), null);
});

test('a javascript: href is stripped but an ordinary one survives', () => {
  const el = parse(
    '<svg xmlns="http://www.w3.org/2000/svg">' +
      '<a href="javascript:alert(1)"><rect/></a>' +
      '<a href="https://example.com"><circle/></a>' +
      '</svg>',
  );
  sanitizeSvg(el);
  const links = Array.from(el.querySelectorAll('a'));
  assert.equal(links[0].getAttribute('href'), null);
  assert.equal(links[1].getAttribute('href'), 'https://example.com');
});

test('ordinary drawing attributes are left alone', () => {
  const el = parse(
    '<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 L1 1" fill="#abc"/></svg>',
  );
  sanitizeSvg(el);
  const p = el.querySelector('path');
  assert.equal(p.getAttribute('d'), 'M0 0 L1 1');
  assert.equal(p.getAttribute('fill'), '#abc');
});
