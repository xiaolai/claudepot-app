// The markdown pipeline, rendered for real.
//
// `renderToStaticMarkup` needs no DOM, so these run in plain node
// against the same configuration the component uses — which is the whole
// reason `mdConfig.js` is JSX-free. Every assertion here is about output
// a transcript could actually produce.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createElement as h, Suspense } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ReactMarkdown from 'react-markdown';

import { MD_COMPONENTS, REHYPE_PLUGINS, REMARK_PLUGINS } from './mdConfig.js';

/** Render a transcript body exactly as the thread does. */
function render(text) {
  return renderToStaticMarkup(
    h(
      ReactMarkdown,
      {
        remarkPlugins: REMARK_PLUGINS,
        rehypePlugins: REHYPE_PLUGINS,
        components: MD_COMPONENTS,
      },
      text,
    ),
  );
}

test('the constructs Claude actually writes come out as elements', () => {
  // Taken from a real transcript on this machine, which is where the
  // defect was found: this rendered as literal asterisks and pipes.
  const html = render('Released. **v0.9.50 is building.**');
  assert.match(html, /<strong>v0\.9\.50 is building\.<\/strong>/);
  assert.doesNotMatch(html, /\*\*/, 'the asterisks survived');

  const heading = render('## The design problem');
  assert.match(heading, /<h2>The design problem<\/h2>/);

  const list = render('- one\n- two');
  assert.match(list, /<ul>/);
  assert.match(list, /<li>one<\/li>/);

  const code = render('```rust\nlet x = 1;\n```');
  assert.match(code, /<pre><code/);
  assert.match(code, /let x = 1;/);

  const inline = render('run `cargo test` first');
  assert.match(inline, /<code>cargo test<\/code>/);
});

test('a GFM table renders as a table inside a scroll wrapper', () => {
  // The worst case on a phone: this was one run-on line of pipes.
  const html = render('| Step | Result |\n|---|---|\n| PR #1320 | 17/17 pass |');
  assert.match(html, /<div class="md-table-scroll"><table>/);
  assert.match(html, /<th[^>]*>Step<\/th>/);
  assert.match(html, /<td[^>]*>17\/17 pass<\/td>/);
  assert.doesNotMatch(html, /\|---\|/, 'the delimiter row leaked as text');
});

test('embedded HTML is escaped, never mounted', () => {
  // The input is model output quoting arbitrary files. Without
  // `rehype-raw` — which is why REHYPE_PLUGINS is empty — react-markdown
  // escapes raw HTML rather than building elements from it.
  //
  // The assertions are about *elements and attributes*, not substrings.
  // A first draft rejected any `onerror=` anywhere and failed on
  // `onerror=&quot;alert(1)&quot;` — the escaped, inert form, i.e. the
  // test rejecting exactly the behaviour it was written to require. An
  // attribute is only real if a bare quote follows the `=`; the escaped
  // form has `&quot;` there and cannot match.
  for (const hostile of [
    '<script>alert(1)</script>',
    '<img src=x onerror="alert(1)">',
    'text <iframe src="https://evil.example"></iframe> more',
    '<div onclick="alert(1)">click</div>',
    '<a href="javascript:alert(1)">x</a>',
  ]) {
    const html = render(hostile);
    assert.doesNotMatch(
      html,
      /<(script|iframe|object|embed|form|style)\b/i,
      `an element was built from: ${hostile}`,
    );
    assert.doesNotMatch(html, /\son[a-z]+\s*=\s*["']/i, `a live handler survived: ${hostile}`);
    assert.doesNotMatch(html, /href\s*=\s*["']javascript:/i, hostile);
    assert.match(html, /&lt;/, `the angle brackets were not escaped: ${hostile}`);
  }
});

test('the escaping assertions are capable of failing', () => {
  // A guard nobody has watched fail is indistinguishable from one that
  // cannot. This builds the markup the checks above must reject, and
  // asserts they would reject it — so enabling raw HTML upstream cannot
  // quietly turn the test above into a tautology.
  const raw = renderToStaticMarkup(
    h('div', { dangerouslySetInnerHTML: { __html: '<iframe src="x" onload="y"></iframe>' } }),
  );
  assert.match(raw, /<(script|iframe|object|embed|form|style)\b/i);
  assert.match(raw, /\son[a-z]+\s*=\s*["']/i);
  assert.doesNotMatch(raw, /&lt;/, 'the sample was escaped, so it proves nothing');
});

test('a mermaid fence dispatches to the diagram renderer, lazily', () => {
  // Asserted by INSPECTING the element rather than rendering it. Two
  // reasons: rendering would resolve the lazy import and pull ~3.4 MB of
  // mermaid into a unit test, and node cannot parse the `.jsx` it points
  // at. The dispatch decision is what this file owns; the drawing is
  // mermaid's.
  const el = MD_COMPONENTS.pre({
    children: h('code', { className: 'language-mermaid' }, 'flowchart TD'),
  });
  // A Suspense boundary whose fallback is the source, so a slow chunk
  // degrades to what the reader had before.
  assert.equal(el.type, Suspense);
  assert.equal(el.props.fallback.type, 'pre');
  assert.equal(el.props.fallback.props.children, 'flowchart TD');
});

test('a non-mermaid fence stays a code block', () => {
  // The dispatch is on the language, not on "is a fence".
  const el = MD_COMPONENTS.pre({
    children: h('code', { className: 'language-rust' }, 'let x = 1;'),
  });
  assert.equal(el.type, 'pre');
});

test('a fence with no language stays a code block', () => {
  const el = MD_COMPONENTS.pre({ children: h('code', {}, 'plain') });
  assert.equal(el.type, 'pre');
});

test('no rehype plugin is configured, and adding one is a decision', () => {
  // The guard that matters over time. `rehype-raw` would turn every
  // assertion above from true into false, in one line, silently.
  assert.equal(REHYPE_PLUGINS.length, 0, 'a rehype plugin was added — was that deliberate?');
  assert.equal(REMARK_PLUGINS.length, 1, 'a remark plugin was added — was that deliberate?');
});

test('a javascript: link is not a link', () => {
  const html = render('[tap me](javascript:alert(1))');
  assert.doesNotMatch(html, /href="javascript:/i, html);
});

test('a real link opens away from the panel and cannot reach back', () => {
  // Navigating the panel itself would drop an authenticated session
  // mid-thread; `noopener` stops the opened page reaching `window.opener`.
  const html = render('see [the docs](https://example.com/x)');
  assert.match(html, /<a href="https:\/\/example\.com\/x"/);
  assert.match(html, /target="_blank"/);
  assert.match(html, /rel="noopener noreferrer"/);
});

test('an image renders as its alt text, not as an image', () => {
  const html = render('![a diagram](https://evil.example/pixel.png)');
  assert.doesNotMatch(html, /<img/i, 'an image element would be a tracking pixel');
  assert.match(html, /class="md-img"/);
  assert.match(html, /a diagram/);
  // The source stays reachable as a tooltip so nothing is hidden.
  assert.match(html, /title="https:\/\/evil\.example\/pixel\.png"/);
});

test('an image with no alt text still says something', () => {
  const html = render('![](https://example.com/x.png)');
  assert.match(html, /image/);
});

test('plain prose with no markdown in it is unchanged', () => {
  // The common case must not acquire stray markup.
  const html = render('Clean. The two pgrep hits are false positives.');
  assert.equal(html, '<p>Clean. The two pgrep hits are false positives.</p>');
});

test('an empty body renders nothing rather than throwing', () => {
  assert.equal(render(''), '');
});
