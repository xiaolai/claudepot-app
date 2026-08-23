// Path and time formatting.
//
// `basename` and `tightPath` are path processing, and
// `.claude/rules/paths.md` requires path code to ship with tests
// covering all four shapes — Unix, Windows drive, UNC, and verbatim —
// on every host OS. They are golden tests with literal expected values
// for exactly the reason that rule gives: the Windows cases have to fail
// on macOS if they regress, not only on a Windows runner.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { ago, basename, bytes, compact, long, modelLabel, short, span, tightPath, until } from './format.js';

test('basename handles all four path shapes', () => {
  assert.equal(basename('/Users/joker/code/claudepot'), 'claudepot');
  assert.equal(basename('C:\\Users\\joker\\code\\claudepot'), 'claudepot');
  assert.equal(basename('\\\\server\\share\\code\\claudepot'), 'claudepot');
  // The verbatim prefix is what `std::fs::canonicalize` returns on
  // Windows. Claude Code never writes it into a `cwd`, but a path that
  // reaches this panel through some other route must not confuse it.
  assert.equal(basename('\\\\?\\C:\\Users\\joker\\code\\claudepot'), 'claudepot');
});

test('basename survives the degenerate inputs', () => {
  assert.equal(basename(''), '');
  assert.equal(basename(null), '');
  assert.equal(basename(undefined), '');
  assert.equal(basename('/'), '/', 'a bare root has no basename to give');
  assert.equal(basename('/Users/joker/code/'), 'code', 'a trailing separator is not a segment');
  assert.equal(basename('C:\\'), 'C:');
  assert.equal(basename('relative/name.txt'), 'name.txt');
});

test('tightPath keeps the basename, which is the identity', () => {
  // Paths read from the end. Clipping the tail would remove the only
  // part that distinguishes two projects under one parent.
  const unix = '/Users/joker/github/xiaolai/myprojects/claudepot-app';
  const tight = tightPath(unix, 24);
  assert.ok(tight.length <= 24, `too long: ${tight}`);
  assert.ok(tight.endsWith('claudepot-app'), tight);
  assert.ok(tight.startsWith('…'), tight);

  const win = 'C:\\Users\\joker\\github\\xiaolai\\myprojects\\claudepot-app';
  assert.ok(tightPath(win, 24).endsWith('claudepot-app'));

  const unc = '\\\\server\\share\\myprojects\\claudepot-app';
  assert.ok(tightPath(unc, 24).endsWith('claudepot-app'));

  const verbatim = '\\\\?\\C:\\Users\\joker\\myprojects\\claudepot-app';
  assert.ok(tightPath(verbatim, 24).endsWith('claudepot-app'));
});

test('tightPath returns a short path untouched', () => {
  assert.equal(tightPath('/tmp/p', 34), '/tmp/p');
  assert.equal(tightPath('', 34), '');
  assert.equal(tightPath(null, 34), '');
});

test('tightPath still ends in the basename when the basename alone is too long', () => {
  const long = `/a/${'x'.repeat(60)}`;
  const out = tightPath(long, 20);
  assert.ok(out.length <= 20, `too long: ${out}`);
  assert.ok(out.startsWith('…'));
});

test('ago and until are not the same measurement', () => {
  const now = Date.parse('2026-08-23T12:00:00Z');
  assert.equal(ago('2026-08-23T11:48:00Z', now), '12m');
  assert.equal(ago('2026-08-23T09:00:00Z', now), '3h');
  assert.equal(ago('2026-08-21T12:00:00Z', now), '2d');
  // `ago` clamps a future instant to zero, which is why the session
  // expiry row cannot use it.
  assert.equal(ago('2026-08-23T13:00:00Z', now), '0s');
  assert.equal(until('2026-08-23T13:00:00Z', now), '1h');
  assert.equal(until('2026-08-23T11:00:00Z', now), null, 'a past instant has no "until"');
  assert.equal(until(null, now), null);
});

test('ago says nothing rather than something wrong', () => {
  assert.equal(ago(null), '—');
  assert.equal(ago('not a date'), '—');
});

test('span is one unit below an hour and two above it', () => {
  assert.equal(span('2026-08-23T12:00:00Z', '2026-08-23T12:14:00Z'), '14m');
  assert.equal(span('2026-08-23T12:00:00Z', '2026-08-23T13:08:00Z'), '1h 08m');
  assert.equal(span('2026-08-23T13:00:00Z', '2026-08-23T12:00:00Z'), null, 'time does not run backwards');
  assert.equal(span(null, '2026-08-23T12:00:00Z'), null);
});

test('short and long agree below a minute and diverge above an hour', () => {
  assert.equal(short(45_000), '45s');
  assert.equal(long(45_000), '45s');
  assert.equal(short(3 * 3600_000 + 5 * 60_000), '3h');
  assert.equal(long(3 * 3600_000 + 5 * 60_000), '3h 05m');
});

test('compact keeps a digit where it distinguishes', () => {
  assert.equal(compact(0), '0');
  assert.equal(compact(999), '999');
  assert.equal(compact(1_500), '1.5k');
  assert.equal(compact(84_120), '84k');
  assert.equal(compact(1_800_000), '1.8M');
  assert.equal(compact(null), null);
});

test('bytes reports binary units, as a filesystem does', () => {
  assert.equal(bytes(512), '512 B');
  assert.equal(bytes(2048), '2 KB');
  assert.equal(bytes(842 * 1024 * 1024), '842 MB');
  assert.equal(bytes(3.1 * 1024 * 1024 * 1024), '3.1 GB');
  assert.equal(bytes(null), null);
});

test('modelLabel drops the prefix and the date stamp', () => {
  assert.equal(modelLabel('claude-opus-5'), 'opus-5');
  assert.equal(modelLabel('claude-haiku-4-5-20251001'), 'haiku-4-5');
  assert.equal(modelLabel(null), null);
});
