import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { toMermaidColor } from './color.js';

/**
 * The panel half of the shared vectors.
 *
 * `src/sections/config/mermaidColor.ts` runs the same file in
 * `src/sections/config/oklchVectors.test.ts`. The two implementations
 * cannot import each other — this app is a separate Vite build — so
 * this file is the only thing that keeps them equal. Same arrangement
 * as `PriceBook::resolve` / `src/costs.ts`.
 *
 * Both agreed on every vector when this was introduced; the point is
 * that they go on agreeing.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const vectorsPath = path.resolve(here, '../../../crates/claudepot-core/testdata/oklch-vectors.json');
const vectors = JSON.parse(fs.readFileSync(vectorsPath, 'utf8'));

test('the shared vectors file is present and non-trivial', () => {
  assert.ok(vectors.cases.length > 10, `only ${vectors.cases.length} vectors`);
});

test('the panel conversion matches every shared vector', () => {
  for (const { input, expected } of vectors.cases) {
    assert.equal(toMermaidColor(input), expected, `input ${JSON.stringify(input)}`);
  }
});
