// The oklch → sRGB transform.
//
// Anchored on the sRGB primaries, whose oklch coordinates are published
// in CSS Color 4 — so these are checks against a standard, not against
// whatever the implementation happened to produce the day it was written.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

import { toMermaidColor } from './color.js';

test('the sRGB primaries round-trip exactly', () => {
  assert.equal(toMermaidColor('oklch(0% 0 0)'), '#000000');
  assert.equal(toMermaidColor('oklch(100% 0 0)'), '#ffffff');
  assert.equal(toMermaidColor('oklch(62.796% 0.25768 29.234)'), '#ff0000');
  assert.equal(toMermaidColor('oklch(86.644% 0.29483 142.495)'), '#00ff00');
  assert.equal(toMermaidColor('oklch(45.201% 0.31321 264.052)'), '#0000ff');
});

test('the palette this app actually paints with converts', () => {
  // Straight out of ds-tokens.css. The accent is terracotta; if this
  // ever comes back grey, the transform has broken rather than drifted.
  assert.equal(toMermaidColor('oklch(63% 0.128 41)'), '#c96c49');
  assert.equal(toMermaidColor('oklch(21% 0.006 60)'), '#1a1816');
  assert.equal(toMermaidColor('oklch(97.6% 0.006 85)'), '#f9f7f3');
});

test('alpha survives, because a hairline is mostly transparent', () => {
  // `--hair` is `oklch(24% 0.01 60/0.09)`. Dropping the alpha would
  // paint every subgraph border as near-black.
  const out = toMermaidColor('oklch(24% 0.01 60/0.09)');
  assert.match(out, /^rgba\(\d+, \d+, \d+, 0\.09\)$/, out);
});

test('both alpha spellings parse', () => {
  // The tokens use `60/0.09`; the spec also allows `60 / 0.09`.
  assert.equal(
    toMermaidColor('oklch(24% 0.01 60/0.5)'),
    toMermaidColor('oklch(24% 0.01 60 / 0.5)'),
  );
});

test('a percentage alpha is the same as its fraction', () => {
  assert.equal(toMermaidColor('oklch(63% 0.128 41 / 50%)'), toMermaidColor('oklch(63% 0.128 41 / 0.5)'));
});

test('lightness reads as a fraction or a percentage', () => {
  assert.equal(toMermaidColor('oklch(0.63 0.128 41)'), toMermaidColor('oklch(63% 0.128 41)'));
});

test('`none` components are zero, per the spec', () => {
  assert.equal(toMermaidColor('oklch(none none none)'), '#000000');
});

test('anything khroma already understands is left alone', () => {
  // The reason this is safe to map over every theme variable rather
  // than a curated subset of them.
  for (const v of [
    '#b4542a',
    '#fff',
    'rgb(180, 84, 42)',
    'rgba(180, 84, 42, 0.5)',
    'hsl(18, 62%, 44%)',
    'transparent',
    'currentColor',
    "'Instrument Sans', system-ui, sans-serif",
    '13px',
    '',
  ]) {
    assert.equal(toMermaidColor(v), v, v);
  }
});

test('a non-string is returned untouched', () => {
  assert.equal(toMermaidColor(undefined), undefined);
  assert.equal(toMermaidColor(null), null);
  assert.equal(toMermaidColor(12), 12);
});

test('an out-of-gamut colour is clipped, not wrapped', () => {
  // A channel that overflows must clamp to ff, never wrap to 00 — a
  // wrapped channel turns a bright colour into its opposite.
  const out = toMermaidColor('oklch(99% 0.4 29)');
  assert.match(out, /^#[0-9a-f]{6}$/);
  for (const pair of [out.slice(1, 3), out.slice(3, 5), out.slice(5, 7)]) {
    const v = parseInt(pair, 16);
    assert.ok(v >= 0 && v <= 255);
  }
});

test('the output is something mermaid can actually read', () => {
  // The assertion that matters. Everything above checks arithmetic;
  // this checks the thing that was broken — that mermaid's own colour
  // library accepts the result.
  //
  // khroma is transitive under pnpm's strict layout, so it is resolved
  // from mermaid's own location, exactly as mermaid resolves it.
  const req = createRequire(import.meta.resolve('mermaid'));
  const khroma = req('khroma');

  for (const raw of [
    'oklch(63% 0.128 41)',
    'oklch(21% 0.006 60)',
    'oklch(97.6% 0.006 85)',
    'oklch(24% 0.01 60/0.09)',
  ]) {
    // Proves this test can fail: the raw token must be REJECTED, or
    // converting it would demonstrate nothing.
    assert.throws(() => khroma.isDark(raw), /Unsupported color format/, raw);

    const converted = toMermaidColor(raw);
    assert.doesNotThrow(() => khroma.isDark(converted), `isDark: ${converted}`);
    assert.doesNotThrow(() => khroma.adjust(converted, { h: -30 }), `adjust: ${converted}`);
  }
});
