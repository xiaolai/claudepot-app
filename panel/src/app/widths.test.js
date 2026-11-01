// The panel fills the window it is given, and so does its text.
//
// `controls.css` used to hold Accounts, Settings and sign-in to 720px and
// 380px at the wide step. The desktop app dropped the same kind of cap
// (`.claude/rules/design.md`, "Pane width"), and its guards cannot see this
// app, so a cap written here would return with nothing red.
//
// jsdom has no layout, so this reads the source. That catches the usual
// spellings, not every one. It does not see a width computed at runtime or by
// `calc`, a percentage, a limit counted in characters such as `tightPath`, a
// fixed height with hidden overflow, or a clip made by two elements: `nowrap` on
// a line inside a parent that hides its overflow.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname;
const INDEX = new URL('../../index.html', import.meta.url).pathname;

const read = (...suffixes) =>
  readdirSync(ROOT, { recursive: true })
    .filter((name) => suffixes.some((s) => name.endsWith(s)) && !name.endsWith('.test.js'))
    .map((name) => [name, readFileSync(join(ROOT, name), 'utf8')]);

const code = (text) =>
  text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|\s)\/\/.*$/gm, '$1');

// Large custom properties that do not hold text to a width: the list column
// beside the thread, the height of the composer, a corner radius, and the
// phone frame of the design-system demo.
// `--dot-w` is the width of a separator dot, not of any text.
const TOKENS = ['--pane-list', '--composer-max', '--composer-cap', '--r-pill', '--frame-w', '--frame-h', '--demo-h', '--dot-w'];

// Controls and icons are small. Text is what needs a hundred pixels or more.
// A number inside `${...}` is worked out at runtime, not a width in the source.
const holdsText = (value) =>
  [...value.replace(/\$\{.*$/, '').matchAll(/(\d+(?:\.\d+)?)(ch|px|rem|em|[dsl]?v(?:[wi]|min|max)|cq(?:[wi]|min|max))?(?![\w.%])/gi)].some(([, n, unit = '']) => {
    const u = unit.toLowerCase();
    if (u === 'ch') return true;
    if (/^(?:[dsl]?v|cq)(?:[wi]|min|max)$/.test(u)) return Number(n) < 100;
    return u === 'em' || u === 'rem' ? Number(n) >= 6 : Number(n) >= 100;
  });

// The rest of a declaration. A comma or a line break ends it only outside
// brackets, so the second argument of `min(100%, 720px)` is still part of the
// value, however a formatter lays it out.
function valueAt(text, from) {
  let depth = 0;
  let i = from;
  for (; i < text.length; i++) {
    const c = text[i];
    if (c === '(') depth++;
    else if (c === ')') depth--;
    else if (c === ';' || c === '}' || ((c === '\n' || c === ',') && depth === 0)) break;
  }
  return text.slice(from, i).trim();
}

const SIZES = /(?<![\w-])(?:width|inline-size|inlineSize|flex-basis|flexBasis|flex|grid-template-columns|gridTemplateColumns|grid-template|gridTemplate|grid-auto-columns|gridAutoColumns|columns)['"\]]*\s*[:=,]\s*/gi;

function capsIn(source) {
  const text = code(source);
  const caps = [];
  for (const m of text.matchAll(/(?:max-(?:width|inline-size)|maxWidth|maxInlineSize)(?![\w-])['"\]]*\s*(:?)\s*/gi)) {
    const value = valueAt(text, m.index + m[0].length);
    // `none` undoes a limit: a Mermaid diagram scrolls rather than shrinks.
    // `max-content` is the text's own width, so it cannot cut the text.
    if (!(m[1] && ['none', 'max-content'].includes(value.replace(/['"]/g, '').toLowerCase()))) caps.push(`${m[0]}${value}`.trim());
  }
  for (const m of text.matchAll(SIZES)) {
    const value = valueAt(text, m.index + m[0].length);
    if (holdsText(value)) caps.push(`${m[0]}${value}`.trim());
  }
  // Only a length: colours, weights and durations are numbers too.
  for (const m of text.matchAll(/(--[\w-]+)\s*:\s*/g)) {
    const value = valueAt(text, m.index + m[0].length);
    const length = /^\d[\d.]*(?:ch|px|rem|em)$/i.test(value) || /^(?:min|max|clamp|calc)\(/i.test(value);
    if (length && holdsText(value) && !TOKENS.includes(m[1])) caps.push(`${m[1]}: ${value}`);
  }
  return caps;
}

// The braces around a position, counting nested ones, so a value holding its
// own `${...}` or `{...}` does not split the block it sits in.
function blockAround(text, at) {
  let start = at;
  for (let depth = 0; start > 0; start--) {
    if (text[start - 1] === '}') depth++;
    else if (text[start - 1] === '{' && depth-- === 0) break;
  }
  let end = at;
  for (let depth = 0; end < text.length; end++) {
    if (text[end] === '{') depth++;
    else if (text[end] === '}' && depth-- === 0) break;
  }
  return text.slice(start, end);
}

// Hidden overflow on an unwrapped line with no ellipsis: the text is cut and
// nothing says so. `flex-wrap: nowrap` is about items, not a line of text.
function clipsIn(source) {
  const text = code(source);
  return [...text.matchAll(/(?<!flex-?wrap['"]?\s*:\s*['"]?)nowrap/gi)]
    .map((m) => blockAround(text, m.index))
    .filter(
      (b) =>
        /overflow(?:-x|-inline|X|Inline)?['"]?\s*:\s*['"]?(?:hidden|clip)/i.test(b) &&
        !/(?:text-overflow|textOverflow)['"]?\s*:\s*['"]?ellipsis/i.test(b),
    );
}

test('the scans recognise the usual ways of writing the fault', () => {
  for (const cap of [
    '.a { max-width: 720px; }',
    '.a{max-width:720px}',
    '.a { max-inline-size: 60ch; }',
    '.a { width: 60ch; }',
    '.a { width: min(720px, 100%); }',
    '.a { flex: 0 0 720px; }',
    '.a { grid-template-columns: 160px 1fr; }',
    ':root { --read-max: 720px; }',
    'const a = { maxWidth: 720 };',
    "const a = { maxWidth: 'var(--meter-md)' };",
    'const a = { maxWidth : 720 };',
    "const a = { ['maxWidth']: 720 };",
    'const a = { maxWidth };',
    "const a = { width: '72ch' };",
    'const a = { flexBasis: 720 };',
    "const a = { gridTemplateColumns: '160px 1fr' };",
    '.a { width: min(100%, 720px); }',
    '.a { width: clamp(10px, 50%, 720px); }',
    '.a { MAX-WIDTH: 720px; }',
    '.a { WIDTH: 720px; }',
    '.a { grid-template-columns: minmax(0, 720px) 1fr; }',
    '.a { grid-template-columns: repeat(2, 400px); }',
    '.a { grid-template: auto / 720px 1fr; }',
    '.a { grid-auto-columns: 720px; }',
    '.a { columns: 40ch; }',
    '.a { width: 50vw; }',
    '.a { width: 50dvw; }',
    '.a { width: 50cqw; }',
    '.a { width: 50vmin; }',
    '.a { width: 50svmin; }',
    '.a { width: 50dvmax; }',
    '.a { width: 50cqmin; }',
    '.a { width: min(\n  100%,\n  720px\n); }',
    '.a { grid-template-columns: repeat(\n  2,\n  400px\n); }',
    ':root { --read: calc(720px); }',
    ':root { --read: min(720px, 100%); }',
    "el.style.width = '720px';",
    "el.style.setProperty('width', '720px');",
  ]) {
    assert.notDeepEqual(capsIn(cap), [], `missed: ${cap}`);
  }
  for (const fine of [
    '.a { max-width: none; }',
    '.a { width: 100%; }',
    '.a { width: 100vw; }',
    '.a { width: 100dvmax; }',
    '.a { width: min(100%, 16px); }',
    'const a = { width: `${Math.min(pct,100)}%` };',
    '.a { width: 16px; }',
    'const a = { flex: 1, width: 18 };',
    '/* max-width: 720px; */',
    '// maxWidth: 720',
  ]) {
    assert.deepEqual(capsIn(fine), [], `flagged: ${fine}`);
  }
  assert.equal(clipsIn("x = { overflow: 'hidden', whiteSpace: 'nowrap' }").length, 1);
  assert.equal(clipsIn('.a { white-space: nowrap; overflow: hidden; }').length, 1);
  assert.equal(clipsIn('.a { white-space: nowrap; overflow: clip; }').length, 1);
  assert.equal(clipsIn('.a { WHITE-SPACE: NOWRAP; OVERFLOW: HIDDEN; }').length, 1);
  assert.equal(clipsIn('.a { text-wrap: nowrap; overflow-inline: hidden; }').length, 1);
  assert.equal(clipsIn("x = { whiteSpace: c ? 'nowrap' : 'normal', overflow: 'hidden' }").length, 1);
  assert.equal(clipsIn("x = { color: `${c}`, whiteSpace: 'nowrap', overflow: 'hidden' }").length, 1);
  assert.equal(clipsIn("x = { color: f({ a: 1 }), whiteSpace: 'nowrap', overflow: 'hidden' }").length, 1);
  assert.equal(clipsIn("x = { flexWrap: 'nowrap', overflow: 'hidden' }").length, 0);
  assert.equal(clipsIn("x = { overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }").length, 0);
});

test('no stylesheet, component or page holds text to a width of its own', () => {
  const files = read('.css', '.jsx', '.js');
  assert.ok(files.some(([name]) => name.endsWith('.css')), 'the scan found no stylesheets');
  assert.ok(files.some(([name]) => name.endsWith('.jsx')), 'the scan found no components');
  files.push(['index.html', readFileSync(INDEX, 'utf8')]);
  const caps = files.flatMap(([name, text]) => capsIn(text).map((cap) => [name, cap]));
  assert.deepEqual(caps, []);
});

test('no block both unwraps and hides a line without an ellipsis', () => {
  const clips = read('.css', '.jsx', '.js').flatMap(([name, text]) => clipsIn(text).map((b) => [name, b.trim()]));
  assert.deepEqual(clips, []);
});

// The row's warning comes first, because on one line what does not fit is dropped. A row that does not wrap squeezes every item instead.
test('a session row wraps its details', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Sessions.jsx'));
  const text = code(source);
  const warning = text.indexOf('had errors</span>');
  assert.ok(warning > 0, 'the warning moved, so this is not looking at the row');
  const row = text.slice(text.lastIndexOf("'mono dots'", warning), warning);
  assert.ok(warning < text.indexOf('{showProject &&', warning), 'the warning must lead the project');
  assert.ok(warning < text.indexOf('bits.map', warning), 'the warning must lead the other details');
  assert.match(text, /<Meta s=\{s\} oneLine \/>/);
  assert.match(row, /flexWrap: 'wrap'/);
  assert.match(row, /display: 'flex'/);
  assert.match(row, /overflowWrap: 'anywhere'/);
});

test('the thread header grows with its wrapped details', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Thread.jsx'));
  const text = code(source);
  const details = text.indexOf('tightPath(session.project_path');
  assert.ok(details > 0, 'the details moved, so this is not looking at the header');
  const header = text.slice(text.lastIndexOf('<header', details), details);
  assert.match(header, /overflowWrap: 'anywhere'/);
  assert.match(header, /minHeight: 'var\(--bar-h-back\)'/);
  assert.doesNotMatch(header, /[{\s]height:/);
});

// `nowrap` here is a clip the scan above cannot see: the card that hides the
// overflow is a different element. A long model label was cut with no ellipsis.
test('an account meter label wraps', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Accounts.jsx'));
  const text = code(source);
  const label = text.indexOf('{m.sub}');
  assert.ok(label > 0, 'the label moved, so this is not looking at it');
  const span = text.slice(text.lastIndexOf('<span', label), label);
  assert.doesNotMatch(span, /nowrap/);
  assert.match(span, /overflowWrap: 'anywhere'/);
});

// The row beside the rail is a flex child of a column, whose default minimum
// height is its content. It has to be zero or no pane inside can scroll.
test('the wide stage gives its panes a height to scroll in', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Panel.jsx'));
  const text = code(source);
  const at = text.indexOf('const stage = wide');
  assert.ok(at > 0, 'the stage moved, so this is not looking at it');
  const opening = text.slice(at, text.indexOf('>', at));
  assert.match(opening, /minHeight: 0/);
});

// `IcoFill` draws outline paths with the stroke off when an icon has no filled
// shape, which leaves only the closed parts of it. Settings became two dots.
test('an active tab icon has a filled shape or is drawn as a line', () => {
  const [[, panel]] = read('.jsx').filter(([name]) => name.endsWith('Panel.jsx'));
  assert.match(code(panel), /on && t\.line \? \(\s*<Ico n=\{t\.ico\}/);
  const icons = readFileSync(join(ROOT, 'vendor/ds-icons.jsx'), 'utf8');
  const filled = icons.slice(icons.indexOf('FPATH'), icons.indexOf('function IcoFill'));
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Panel.jsx'));
  const tabs = code(source).match(/const TABS = \[([\s\S]*?)\];/)[1];
  const entries = [...tabs.matchAll(/\{[^}]*\}/g)].map((m) => m[0]);
  assert.ok(entries.length > 1, 'the tabs moved, so this is not looking at them');
  for (const entry of entries) {
    const name = entry.match(/ico: '(\w+)'/)[1];
    assert.ok(/line: true/.test(entry) || new RegExp(`\\b${name}:`).test(filled), `${name} has no filled shape`);
  }
});

// Over the filled icon the badge hid half the shape and left a wedge.
test('the tab badge sits beside its icon', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Panel.jsx'));
  const text = code(source);
  const at = text.indexOf('<Badge n={badgeCount(n)} />');
  assert.ok(at > 0, 'the badge moved, so this is not looking at it');
  const wrapper = text.slice(text.lastIndexOf('<span', at), at);
  assert.match(wrapper, /left: n > 99 \? 'calc\(100% - var\(--s1\)\)' : '100%'/);
  assert.doesNotMatch(wrapper, /right:/);
});

// The list is a share of the stage, so a wider window gives its text more room.
test('the session list beside the thread takes a share of the stage', () => {
  const [[, source]] = read('.jsx').filter(([name]) => name.endsWith('Panel.jsx'));
  assert.match(code(source), /view === 'sessions' \? 'max\(var\(--pane-list\), 30%\)'/);
});

const ruleIn = (css, selector) => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const rule = css.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`));
  assert.ok(rule, `${selector} is gone`);
  return rule[1];
};

// The dot must not be part of the text, or a wrapped line can open with one.
test('wrapping details keep their separators out of the text', () => {
  for (const [name, needle, className] of [
    ['Sessions.jsx', 'had errors</span>', 'mono dots'],
    ['Thread.jsx', 'tightPath(session.project_path', 'mono selectable dots'],
  ]) {
    const [[, source]] = read('.jsx').filter(([n]) => n.endsWith(name));
    const text = code(source);
    const at = text.indexOf(needle);
    assert.ok(at > 0, `${name} moved, so this is not looking at the details`);
    const start = text.lastIndexOf(className, at);
    assert.ok(start > 0, `${name} details lost the dots class`);
    const details = text.slice(start, text.indexOf('</div>', at));
    assert.match(details, /display: 'flex'/);
    assert.match(details, /flexWrap: 'wrap'/);
    assert.doesNotMatch(details, /·/);
  }
  const css = readFileSync(join(ROOT, 'controls.css'), 'utf8');
  const dots = ruleIn(css, '.dots');
  assert.match(dots, /margin-left: calc\(var\(--dot-w\) \* -1\)/);
  assert.match(dots, /clip-path: inset\(0 0 0 var\(--dot-w\)\)/);
  assert.match(ruleIn(css, '.dots > *'), /padding-left: var\(--dot-w\)/);
  const dot = ruleIn(css, '.dots > *::before');
  assert.match(dot, /content: '·'/);
  assert.match(dot, /position: absolute/);
  assert.match(dot, /left: 0/);
  // An empty item takes the clipped first place, and the next dot opens the line.
  assert.match(ruleIn(css, '.dots > :empty'), /display: none/);
});

// A plain space lets the line break before the dot. `vendor/` stays byte-identical
// to the design system; its one dot is in "Offline · 3 queued", too short to wrap.
test('a separator inside one run of text cannot start a line', () => {
  const dots = read('.jsx', '.js').filter(([name]) => !name.startsWith('vendor/')).flatMap(([name, source]) => {
    const text = code(source);
    return [...text.matchAll(/·/g)].map((m) => [name, text.slice(Math.max(0, m.index - 6), m.index)]);
  });
  assert.ok(dots.length >= 3, 'the separators moved, so this is not looking at them');
  for (const [name, before] of dots) assert.ok(before.endsWith('\\u00a0'), `${name}: ${before}·`);
});

test('the composer placeholder stays on one line', () => {
  const [[, source]] = read('.jsx').filter(([n]) => n.endsWith('Thread.jsx'));
  const text = code(source);
  assert.match(text, /className="one-line-hint"/);
  const at = text.indexOf('placeholder={');
  assert.ok(at > 0, 'the placeholder moved, so this is not looking at it');
  const placeholder = text.slice(at, text.indexOf('style={{', at));
  // A reason is written out below the field. Here it could only be cut.
  assert.match(placeholder, /disabled\s*\?\s*'Unavailable'/);
  const hints = [...placeholder.matchAll(/'([^']*)'/g)].map((m) => m[1]);
  assert.ok(hints.length >= 4, 'the placeholder moved, so this is not looking at it');
  for (const hint of hints) assert.ok(hint.length <= 24, `too long for the field: ${hint}`);
  const css = readFileSync(join(ROOT, 'controls.css'), 'utf8');
  const rule = css.match(/\.one-line-hint:placeholder-shown,\s*\.one-line-hint::placeholder\s*\{([^}]*)\}/);
  assert.ok(rule, 'the placeholder rule is gone');
  assert.match(rule[1], /white-space: nowrap/);
  assert.match(rule[1], /overflow: hidden/);
  assert.match(rule[1], /text-overflow: ellipsis/);
});

test('a row holding a segmented control can wrap it', () => {
  // Seg cannot shrink and List clips, so the page gains no scroll and nothing
  // but this notices a control cut off at the card's edge.
  const rows = read('.jsx')
    .filter(([n]) => !n.includes('vendor'))
    .flatMap(([n, source]) => {
      const text = code(source);
      return [...text.matchAll(/<Seg\b/g)].map((m) => [n, text.slice(text.lastIndexOf('<Item', m.index), m.index)]);
    });
  assert.ok(rows.length >= 2, 'the controls moved, so this is not looking at them');
  for (const [name, row] of rows) {
    assert.match(row, /^<Item[^>]*flexWrap: 'wrap'/, `${name}: the row cannot wrap`);
    assert.match(row, /flex: '1 1 auto'/, `${name}: a label with no basis always fits, so the row never wraps`);
  }
});
